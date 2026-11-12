/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Incoming call approval over MIPC for MediaTek CCCI modems.
 *
 * The modem does not alert an incoming call by itself: it first asks the
 * AP with the MIPC command CALL_APPROVE_INCOMING_CMD, which goes to the MIPC
 * client that registered for it, and waits for CALL_APPROVE_INCOMING_RSP.
 * No AT URC (RING, +CRING, +CLIP, +EAIC) is sent on the AT port, neither
 * before nor after the approval. So this opens a MIPC channel of its own,
 * registers for the command, approves every incoming call and has the
 * voice interface reload the call list (+CLCC), which reports the call as
 * ringing; answering and hanging up are plain ATA/ATH on the AT port.
 *
 * Neither does the modem report call state changes on the AT port (no NO
 * CARRIER when the other side hangs up), so this also registers for the
 * MIPC call indications and reloads the call list on each of them.
 *
 * MIPC message: u32 magic, u32 0, u8 SIM/PS id, u8 0, u16 message id,
 * u16 transaction id, u16 payload length, then TLVs: u16 type, u16 length,
 * value, each TLV padded to 8 bytes. As the stock libmipc.so/
 * libmtkmipc-ril.so of the phone do it.
 *
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#include <config.h>

#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <unistd.h>

#include <ModemManager.h>
#define _LIBMM_INSIDE_MM
#include <libmm-glib.h>

#include "mm-mtk-ccci-mipc.h"
#include "mm-log-object.h"

#define MIPC_MAGIC                       0x24541984
#define MIPC_HDR_LEN                     16
#define MIPC_SIM_PS_ALL                  0xff
#define MIPC_SIM_PS_SIM0                 0x01

#define MIPC_SYS_INTERNAL_OPEN_REQ       0x0301
#define MIPC_SYS_REGISTER_IND_REQ        0x0307
#define MIPC_SYS_REGISTER_CMD_REQ        0x030b
#define MIPC_CALL_STATUS_IND             0x4901
#define MIPC_CALL_EVENT_IND              0x4902
#define MIPC_CALL_IND_MASK               0xff00
#define MIPC_CALL_IND                    0x4900
#define MIPC_CALL_APPROVE_INCOMING_CMD   0x8901
#define MIPC_CALL_APPROVE_INCOMING_RSP   0x8902

/* CALL_APPROVE_INCOMING_CMD */
#define TLV_CMD_CALL_ID                  0x100
#define TLV_CMD_SEQ_NO                   0x103
/* CALL_APPROVE_INCOMING_RSP */
#define TLV_RSP_IS_APPROVE               0x100
#define TLV_RSP_CALL_ID                  0x102
#define TLV_RSP_SEQ_NO                   0x103

/* give the modem a moment to list the approved call in +CLCC, and merge
 * the bursts of indications of one change */
#define RELOAD_DELAY_MS                  300

struct _MMMtkCcciMipc {
    MMIfaceModemVoice *modem;   /* not owned: the modem owns us */
    gint               fd;
    GIOChannel        *channel;
    guint              watch_id;
    guint              reload_id;
    guint16            txid;
};

static void
put_u16 (GByteArray *msg, guint16 v)
{
    guint16 le = GUINT16_TO_LE (v);

    g_byte_array_append (msg, (const guint8 *) &le, 2);
}

static void
put_u32 (GByteArray *msg, guint32 v)
{
    guint32 le = GUINT32_TO_LE (v);

    g_byte_array_append (msg, (const guint8 *) &le, 4);
}

static void
put_tlv (GByteArray   *msg,
         guint16       type,
         const guint8 *value,
         guint16       len)
{
    static const guint8 zero[8] = { 0 };
    guint               pad = (8 - (4 + len) % 8) % 8;

    put_u16 (msg, type);
    put_u16 (msg, len);
    g_byte_array_append (msg, value, len);
    g_byte_array_append (msg, zero, pad);
}

static GByteArray *
msg_new (guint8  sim_ps,
         guint16 id,
         guint16 txid)
{
    GByteArray *msg;
    guint8      b[2] = { sim_ps, 0 };

    msg = g_byte_array_new ();
    put_u32 (msg, MIPC_MAGIC);
    put_u32 (msg, 0);
    g_byte_array_append (msg, b, 2);
    put_u16 (msg, id);
    put_u16 (msg, txid);
    put_u16 (msg, 0); /* payload length, set by msg_send() */
    return msg;
}

static void
msg_send (MMMtkCcciMipc *self,
          GByteArray    *msg)
{
    guint16 len = GUINT16_TO_LE (msg->len - MIPC_HDR_LEN);

    memcpy (msg->data + 14, &len, 2);
    if (write (self->fd, msg->data, msg->len) != (gssize) msg->len)
        mm_obj_warn (self->modem, "MIPC: cannot send message %#06x: %s",
                     GUINT16_FROM_LE (*(guint16 *) (msg->data + 10)), g_strerror (errno));
    g_byte_array_unref (msg);
}

/* Value of a TLV of a received message, NULL if missing */
static const guint8 *
msg_tlv (const guint8 *buf,
         gsize         len,
         guint16       type,
         guint16      *tlv_len)
{
    gsize end, off = MIPC_HDR_LEN;

    end = MIPC_HDR_LEN + (buf[14] | buf[15] << 8);
    if (end > len)
        end = len;
    while (off + 4 <= end) {
        guint16 t = buf[off] | buf[off + 1] << 8;
        guint16 l = buf[off + 2] | buf[off + 3] << 8;

        if (off + 4 + l > end)
            break;
        if (t == type) {
            *tlv_len = l;
            return buf + off + 4;
        }
        off += 4 + l + (8 - (4 + l) % 8) % 8;
    }
    return NULL;
}

static gboolean
msg_tlv_u32 (const guint8 *buf,
             gsize         len,
             guint16       type,
             guint32      *value)
{
    const guint8 *v;
    guint16       l = 0;

    v = msg_tlv (buf, len, type, &l);
    if (!v || l < 4)
        return FALSE;
    *value = v[0] | v[1] << 8 | v[2] << 16 | (guint32) v[3] << 24;
    return TRUE;
}

static gboolean
reload_calls (MMMtkCcciMipc *self)
{
    self->reload_id = 0;
    mm_iface_modem_voice_reload_all_calls (self->modem, NULL, NULL);
    return G_SOURCE_REMOVE;
}

static void
schedule_reload (MMMtkCcciMipc *self)
{
    if (!self->reload_id)
        self->reload_id = g_timeout_add (RELOAD_DELAY_MS, (GSourceFunc) reload_calls, self);
}

static void
approve_incoming_call (MMMtkCcciMipc *self,
                       const guint8  *buf,
                       gsize          len)
{
    GByteArray *rsp;
    guint32     call_id, seq_no;
    guint8      sim_ps, approve = 1;

    if (!msg_tlv_u32 (buf, len, TLV_CMD_CALL_ID, &call_id) ||
        !msg_tlv_u32 (buf, len, TLV_CMD_SEQ_NO, &seq_no)) {
        mm_obj_warn (self->modem, "MIPC: malformed incoming call approval request");
        return;
    }
    mm_obj_msg (self->modem, "MIPC: approving incoming call %u (seq %u)", call_id, seq_no);

    /* the reply carries the SIM id only (stock: slotIdChangeToMipcSlotId) */
    sim_ps = buf[8] & 0x0f;
    if (!sim_ps)
        sim_ps = MIPC_SIM_PS_SIM0;
    rsp = msg_new (sim_ps, MIPC_CALL_APPROVE_INCOMING_RSP, buf[12] | buf[13] << 8);
    call_id = GUINT32_TO_LE (call_id);
    seq_no = GUINT32_TO_LE (seq_no);
    put_tlv (rsp, TLV_RSP_IS_APPROVE, &approve, 1);
    put_tlv (rsp, TLV_RSP_CALL_ID, (const guint8 *) &call_id, 4);
    put_tlv (rsp, TLV_RSP_SEQ_NO, (const guint8 *) &seq_no, 4);
    msg_send (self, rsp);
    schedule_reload (self);
}

static gboolean
mipc_readable (GIOChannel    *channel,
               GIOCondition   condition,
               MMMtkCcciMipc *self)
{
    guint8  buf[4096];
    gssize  n;
    guint16 id;

    if (condition & (G_IO_ERR | G_IO_HUP | G_IO_NVAL)) {
        mm_obj_warn (self->modem, "MIPC: channel closed");
        self->watch_id = 0;
        return G_SOURCE_REMOVE;
    }

    /* the CCCI port returns one message per read */
    n = read (self->fd, buf, sizeof (buf));
    if (n < 0) {
        if (errno != EAGAIN && errno != EINTR)
            mm_obj_warn (self->modem, "MIPC: read failed: %s", g_strerror (errno));
        return G_SOURCE_CONTINUE;
    }
    if (n < MIPC_HDR_LEN ||
        (buf[0] | buf[1] << 8 | buf[2] << 16 | (guint32) buf[3] << 24) != MIPC_MAGIC) {
        mm_obj_dbg (self->modem, "MIPC: ignoring %zd bytes", n);
        return G_SOURCE_CONTINUE;
    }

    id = buf[10] | buf[11] << 8;
    if (id == MIPC_CALL_APPROVE_INCOMING_CMD)
        approve_incoming_call (self, buf, n);
    else if ((id & MIPC_CALL_IND_MASK) == MIPC_CALL_IND)
        schedule_reload (self);
    else {
        guint32 result = 0;

        msg_tlv_u32 (buf, n, 0, &result);
        mm_obj_dbg (self->modem, "MIPC: message %#06x (result %u)", id, result);
    }
    return G_SOURCE_CONTINUE;
}

MMMtkCcciMipc *
mm_mtk_ccci_mipc_new (MMIfaceModemVoice  *modem,
                      const gchar        *path,
                      GError            **error)
{
    MMMtkCcciMipc *self;
    GByteArray    *msg;
    g_autofree gchar *name = NULL;
    guint32        version = GUINT32_TO_LE (2);
    guint16        cmd = GUINT16_TO_LE (MIPC_CALL_APPROVE_INCOMING_CMD);
    guint16        inds[] = { GUINT16_TO_LE (MIPC_CALL_STATUS_IND),
                              GUINT16_TO_LE (MIPC_CALL_EVENT_IND) };
    guint          i;
    guint8         one = 1;
    gint           fd;

    fd = open (path, O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        g_set_error (error, G_IO_ERROR, g_io_error_from_errno (errno),
                     "cannot open %s: %s", path, g_strerror (errno));
        return NULL;
    }

    self = g_new0 (MMMtkCcciMipc, 1);
    self->modem = modem;
    self->fd = fd;
    self->txid = 1;
    self->channel = g_io_channel_unix_new (fd);
    self->watch_id = g_io_add_watch (self->channel, G_IO_IN | G_IO_ERR | G_IO_HUP,
                                     (GIOFunc) mipc_readable, self);

    /* open a MIPC session named like the stock tty of this channel
     * (/dev/ccci_mipc0 -> /dev/ttyCMIPC0) */
    name = g_strdup_printf ("/dev/ttyCMIPC%c", path[strlen (path) - 1]);
    msg = msg_new (MIPC_SIM_PS_ALL, MIPC_SYS_INTERNAL_OPEN_REQ, self->txid++);
    put_tlv (msg, 0x100, (const guint8 *) &version, 4);
    put_tlv (msg, 0x101, (const guint8 *) name, strlen (name) + 1);
    put_tlv (msg, 0x102, &one, 1);
    msg_send (self, msg);

    msg = msg_new (MIPC_SIM_PS_ALL, MIPC_SYS_REGISTER_CMD_REQ, self->txid++);
    put_tlv (msg, 0x100, (const guint8 *) &cmd, 2);
    msg_send (self, msg);

    /* indications are registered per SIM */
    for (i = 0; i < G_N_ELEMENTS (inds); i++) {
        msg = msg_new (MIPC_SIM_PS_SIM0, MIPC_SYS_REGISTER_IND_REQ, self->txid++);
        put_tlv (msg, 0x100, (const guint8 *) &inds[i], 2);
        msg_send (self, msg);
    }

    mm_obj_msg (modem, "MIPC: approving incoming calls over %s", path);
    return self;
}

void
mm_mtk_ccci_mipc_free (MMMtkCcciMipc *self)
{
    if (!self)
        return;
    if (self->reload_id)
        g_source_remove (self->reload_id);
    if (self->watch_id)
        g_source_remove (self->watch_id);
    g_io_channel_unref (self->channel);
    close (self->fd);
    g_free (self);
}
