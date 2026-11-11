/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Bearer of MediaTek modems behind the mainline CCCI driver (mtk-ccci).
 *
 * The data path of these modems is opened by the AP ("NetAgent" on
 * Android). After a PDP context is activated, the modem reports the network
 * interface it put the context on:
 *
 *   +EIF: <id>, ifup, <type>, <ipv4>, <ipv6 link-local as hex bytes>, 0
 *   +EIF: <id>, mtu, <mtu>
 *   +EIF: <id>, ifchange
 *
 * and keeps the data path closed until the AP confirms it with
 * AT+EIF=<id>,"ifst","up",<type>. The interface is ccmni<id % 100>, and it
 * changes from one activation to the next (101, 202, 301, ...). On
 * deactivation the modem reports "+EIF: <id>, ifdown, 0", which needs no
 * answer. AT+EIFCONFIG=3 enables the reports.
 *
 * The IPv4 address only comes in +EIF (+CGPADDR is not supported and
 * +CGCONTRDP leaves it empty); IPv6 is configured by SLAAC with the
 * interface identifier of the link-local address the modem reports. DNS
 * servers come from +CGCONTRDP, IPv6 ones as 16 dotted decimal bytes.
 *
 * On LTE the default bearer of the attach is context 0; another context to
 * the same APN is refused, so a refused activation falls back to context 0.
 *
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#include <config.h>

#include <stdlib.h>
#include <string.h>
#include <arpa/inet.h>

#include <ModemManager.h>
#define _LIBMM_INSIDE_MM
#include <libmm-glib.h>

#include "mm-broadband-bearer-mtk-ccci.h"
#include "mm-base-modem-at.h"
#include "mm-log-object.h"
#include "mm-modem-helpers.h"
#include "mm-bind.h"

#define EIF_REGEX      "\\r\\n\\+EIF:\\s*(\\d+),\\s*([a-z]+)([^\\r\\n]*)\\r\\n"
#define IFUP_TIMEOUT   30 /* seconds from the activation OK to +EIF ifup */

struct _MMBroadbandBearerMtkCcciPrivate {
    MMPortSerialAt *primary;   /* port with our +EIF handler */
    GRegex         *eif_regex;

    /* connection attempt */
    GTask    *connect_pending;
    guint     connect_timeout_id;
    gboolean  activated;       /* +CGACT=1 answered OK */
    gboolean  confirming;      /* AT+EIF ... "ifst","up" sent */
    gboolean  disconnecting;

    /* the context and its interface */
    guint     cid;
    gint      eif_id;          /* -1: no interface up */
    guint     eif_type;
    gchar    *ipv4;
    gchar    *ipv6;
    guint     mtu;
};

G_DEFINE_TYPE_WITH_PRIVATE (MMBroadbandBearerMtkCcci, mm_broadband_bearer_mtk_ccci, MM_TYPE_BROADBAND_BEARER)

/*****************************************************************************/
/* Address helpers */

/* "a.b.c.d" (IPv4) or 16 dotted decimal bytes (IPv6), as +CGCONTRDP has */
static gchar *
dotted_to_address (const gchar *field, gboolean *is_ipv6)
{
    g_auto(GStrv) parts = NULL;
    guint8        bytes[16];
    gchar         buf[INET6_ADDRSTRLEN];
    guint         n, i;

    parts = g_strsplit (field, ".", -1);
    n = g_strv_length (parts);
    if (n != 4 && n != 16)
        return NULL;
    for (i = 0; i < n; i++) {
        gchar  *end;
        gulong  v = strtoul (parts[i], &end, 10);

        if (end == parts[i] || *end || v > 255)
            return NULL;
        bytes[i] = v;
    }
    for (i = 0; i < n && !bytes[i]; i++)
        ;
    if (i == n)
        return NULL; /* unspecified */
    if (!inet_ntop (n == 4 ? AF_INET : AF_INET6, bytes, buf, sizeof (buf)))
        return NULL;
    *is_ipv6 = (n == 16);
    return g_strdup (buf);
}

/* "FE:80:00:...:D8" (16 hex bytes), as +EIF has */
static gchar *
hex_bytes_to_ipv6 (const gchar *field)
{
    g_auto(GStrv) parts = NULL;
    guint8        bytes[16];
    gchar         buf[INET6_ADDRSTRLEN];
    guint         i;
    gboolean      any = FALSE;

    parts = g_strsplit (field, ":", -1);
    if (g_strv_length (parts) != 16)
        return NULL;
    for (i = 0; i < 16; i++) {
        gchar  *end;
        gulong  v = strtoul (parts[i], &end, 16);

        if (end == parts[i] || *end || v > 255)
            return NULL;
        bytes[i] = v;
        any |= (v != 0);
    }
    if (!any || !inet_ntop (AF_INET6, bytes, buf, sizeof (buf)))
        return NULL;
    return g_strdup (buf);
}

/* DNS servers of all +CGCONTRDP lines (fields 6 and 7) */
static void
parse_cgcontrdp_dns (const gchar *response,
                     GPtrArray   *dns4,
                     GPtrArray   *dns6)
{
    g_auto(GStrv) lines = NULL;
    guint         i;

    lines = g_strsplit_set (response, "\r\n", -1);
    for (i = 0; lines[i]; i++) {
        g_auto(GStrv)  fields = NULL;
        const gchar   *line;
        guint          f;

        if (!g_str_has_prefix (lines[i], "+CGCONTRDP:"))
            continue;
        line = mm_strip_tag (lines[i], "+CGCONTRDP:");
        fields = g_strsplit (line, ",", -1);
        for (f = 5; f <= 6 && f < g_strv_length (fields); f++) {
            g_autofree gchar *addr = NULL;
            gboolean          v6 = FALSE;

            g_strstrip (fields[f]);
            addr = dotted_to_address (mm_strip_quotes (fields[f]), &v6);
            if (addr)
                g_ptr_array_add (v6 ? dns6 : dns4, g_steal_pointer (&addr));
        }
    }
}

/*****************************************************************************/
/* Connection attempt */

typedef struct {
    MMBaseModem    *modem;
    MMPortSerialAt *primary;
} Dial3gppContext;

static void
dial_3gpp_context_free (Dial3gppContext *ctx)
{
    g_clear_object (&ctx->primary);
    g_clear_object (&ctx->modem);
    g_slice_free (Dial3gppContext, ctx);
}

static MMPort *
dial_3gpp_finish (MMBroadbandBearer  *self,
                  GAsyncResult       *res,
                  GError            **error)
{
    return MM_PORT (g_task_propagate_pointer (G_TASK (res), error));
}

static void
ifup_reset (MMBroadbandBearerMtkCcci *self)
{
    self->priv->activated = FALSE;
    self->priv->confirming = FALSE;
    self->priv->eif_id = -1;
    self->priv->eif_type = 0;
    self->priv->mtu = 0;
    g_clear_pointer (&self->priv->ipv4, g_free);
    g_clear_pointer (&self->priv->ipv6, g_free);
}

/* Fail the pending connection attempt and deactivate the context */
static void
connect_fail (MMBroadbandBearerMtkCcci *self,
              GError                   *error)
{
    GTask           *task;
    Dial3gppContext *ctx;
    g_autofree gchar *cmd = NULL;

    task = g_steal_pointer (&self->priv->connect_pending);
    g_assert (task);
    ctx = g_task_get_task_data (task);

    if (self->priv->connect_timeout_id) {
        g_source_remove (self->priv->connect_timeout_id);
        self->priv->connect_timeout_id = 0;
    }
    if (self->priv->activated) {
        cmd = g_strdup_printf ("+CGACT=0,%u", self->priv->cid);
        mm_base_modem_at_command_full (ctx->modem, MM_IFACE_PORT_AT (ctx->primary), cmd,
                                       MM_BASE_BEARER_DEFAULT_DISCONNECTION_TIMEOUT,
                                       FALSE, FALSE, NULL, NULL, NULL);
    }
    ifup_reset (self);
    g_task_return_error (task, error);
    g_object_unref (task);
}

static void
ifst_up_ready (MMBaseModem              *modem,
               GAsyncResult             *res,
               MMBroadbandBearerMtkCcci *self)
{
    GTask            *task;
    GError           *error = NULL;
    MMPort           *data;
    g_autofree gchar *name = NULL;

    task = self->priv->connect_pending;
    if (!task) {
        mm_base_modem_at_command_full_finish (modem, res, NULL);
        goto out;
    }

    if (!mm_base_modem_at_command_full_finish (modem, res, &error)) {
        g_prefix_error (&error, "couldn't confirm interface %d: ", self->priv->eif_id);
        connect_fail (self, error);
        goto out;
    }

    name = g_strdup_printf ("ccmni%d", self->priv->eif_id % 100);
    data = mm_base_modem_peek_port (modem, name);
    if (!data || mm_port_get_port_type (data) != MM_PORT_TYPE_NET) {
        connect_fail (self, g_error_new (MM_CORE_ERROR, MM_CORE_ERROR_NOT_FOUND,
                                         "modem put the context on %s, which is not one of its net ports", name));
        goto out;
    }

    if (g_task_get_cancellable (task) && g_cancellable_is_cancelled (g_task_get_cancellable (task))) {
        connect_fail (self, g_error_new (G_IO_ERROR, G_IO_ERROR_CANCELLED, "connection attempt cancelled"));
        goto out;
    }

    mm_obj_info (self, "context %u is up on %s (+EIF id %d): %s%s%s",
                 self->priv->cid, name, self->priv->eif_id,
                 self->priv->ipv4 ? self->priv->ipv4 : "",
                 self->priv->ipv4 && self->priv->ipv6 ? ", " : "",
                 self->priv->ipv6 ? self->priv->ipv6 : "");

    self->priv->connect_pending = NULL;
    g_task_return_pointer (task, g_object_ref (data), g_object_unref);
    g_object_unref (task);

out:
    g_object_unref (self);
}

/* +EIF ifup arrived and +CGACT answered: confirm the interface */
static void
ifup_confirm (MMBroadbandBearerMtkCcci *self)
{
    Dial3gppContext  *ctx;
    g_autofree gchar *cmd = NULL;

    if (self->priv->confirming || !self->priv->activated || self->priv->eif_id < 0)
        return;
    self->priv->confirming = TRUE;

    if (self->priv->connect_timeout_id) {
        g_source_remove (self->priv->connect_timeout_id);
        self->priv->connect_timeout_id = 0;
    }

    ctx = g_task_get_task_data (self->priv->connect_pending);
    cmd = g_strdup_printf ("+EIF=%d,\"ifst\",\"up\",%u", self->priv->eif_id, self->priv->eif_type);
    mm_base_modem_at_command_full (ctx->modem, MM_IFACE_PORT_AT (ctx->primary), cmd, 10,
                                   FALSE, FALSE, NULL,
                                   (GAsyncReadyCallback) ifst_up_ready,
                                   g_object_ref (self));
}

static gboolean
ifup_timed_out_cb (MMBroadbandBearerMtkCcci *self)
{
    self->priv->connect_timeout_id = 0;
    connect_fail (self, g_error_new (MM_CORE_ERROR, MM_CORE_ERROR_TIMEOUT,
                                     "context %u activated, but the modem reported no interface (+EIF ifup)",
                                     self->priv->cid));
    return G_SOURCE_REMOVE;
}

static void activate (MMBroadbandBearerMtkCcci *self);

static void
activate_ready (MMBaseModem              *modem,
                GAsyncResult             *res,
                MMBroadbandBearerMtkCcci *self)
{
    GError *error = NULL;

    if (!self->priv->connect_pending) {
        mm_base_modem_at_command_full_finish (modem, res, NULL);
        goto out;
    }

    if (!mm_base_modem_at_command_full_finish (modem, res, &error)) {
        if (self->priv->cid != 0) {
            mm_obj_info (self, "context %u refused (%s), using the default bearer of the attach (context 0)",
                         self->priv->cid, error->message);
            g_clear_error (&error);
            self->priv->cid = 0;
            activate (self);
            goto out;
        }
        connect_fail (self, error);
        goto out;
    }

    self->priv->activated = TRUE;
    if (self->priv->eif_id >= 0)
        ifup_confirm (self); /* the +EIF came before the OK */
    else
        self->priv->connect_timeout_id = g_timeout_add_seconds (IFUP_TIMEOUT,
                                                                (GSourceFunc) ifup_timed_out_cb,
                                                                self);
out:
    g_object_unref (self);
}

static void
activate (MMBroadbandBearerMtkCcci *self)
{
    Dial3gppContext  *ctx;
    g_autofree gchar *cmd = NULL;

    ctx = g_task_get_task_data (self->priv->connect_pending);
    cmd = g_strdup_printf ("+CGACT=1,%u", self->priv->cid);
    mm_base_modem_at_command_full (ctx->modem, MM_IFACE_PORT_AT (ctx->primary), cmd,
                                   MM_BASE_BEARER_DEFAULT_CONNECTION_TIMEOUT,
                                   FALSE, FALSE, NULL,
                                   (GAsyncReadyCallback) activate_ready,
                                   g_object_ref (self));
}

static void
eifconfig_ready (MMBaseModem              *modem,
                 GAsyncResult             *res,
                 MMBroadbandBearerMtkCcci *self)
{
    g_autoptr(GError) error = NULL;

    if (!mm_base_modem_at_command_full_finish (modem, res, &error))
        mm_obj_warn (self, "couldn't enable +EIF reports: %s", error->message);
    if (self->priv->connect_pending)
        activate (self);
    g_object_unref (self);
}

/*****************************************************************************/
/* +EIF reports */

static void
eif_received (MMPortSerialAt           *port,
              GMatchInfo               *match_info,
              MMBroadbandBearerMtkCcci *self)
{
    g_autofree gchar *id_str = NULL;
    g_autofree gchar *what = NULL;
    g_autofree gchar *rest = NULL;
    g_auto(GStrv)     args = NULL;
    gint              id;
    guint             i;

    id_str = g_match_info_fetch (match_info, 1);
    what = g_match_info_fetch (match_info, 2);
    rest = g_match_info_fetch (match_info, 3);
    id = atoi (id_str);
    /* rest is ", a, b, ..." */
    args = g_strsplit (rest, ",", -1);
    for (i = 0; args[i]; i++)
        g_strstrip (args[i]);

    mm_obj_dbg (self, "+EIF %d %s%s", id, what, rest);

    if (g_str_equal (what, "ifup")) {
        if (!self->priv->connect_pending || self->priv->eif_id >= 0) {
            mm_obj_dbg (self, "ignoring +EIF ifup %d: no connection attempt waiting for it", id);
            return;
        }
        if (g_strv_length (args) < 4) {
            mm_obj_warn (self, "couldn't parse +EIF ifup%s", rest);
            return;
        }
        self->priv->eif_id = id;
        self->priv->eif_type = atoi (args[1]);
        self->priv->ipv4 = (args[2][0] && !g_str_equal (args[2], "0.0.0.0")) ? g_strdup (args[2]) : NULL;
        self->priv->ipv6 = hex_bytes_to_ipv6 (args[3]);
        ifup_confirm (self);
        return;
    }

    if (id != self->priv->eif_id)
        return;

    if (g_str_equal (what, "mtu")) {
        if (args[0] && args[1])
            self->priv->mtu = atoi (args[1]);
        return;
    }

    if (g_str_equal (what, "ifdown")) {
        gboolean connected;

        connected = !self->priv->connect_pending && !self->priv->disconnecting;
        ifup_reset (self);
        if (connected) {
            mm_obj_info (self, "modem took the interface down (+EIF %d ifdown)", id);
            mm_base_bearer_report_connection_status (MM_BASE_BEARER (self),
                                                     MM_BEARER_CONNECTION_STATUS_DISCONNECTED);
        }
    }
}

static void
eif_handler_set (MMBroadbandBearerMtkCcci *self,
                 MMPortSerialAt           *primary)
{
    if (self->priv->primary == primary)
        return;
    /* Leave a NULL handler on the old port: the URCs are still swallowed */
    if (self->priv->primary) {
        mm_port_serial_at_add_unsolicited_msg_handler (self->priv->primary, self->priv->eif_regex,
                                                       NULL, NULL, NULL);
        g_clear_object (&self->priv->primary);
    }
    if (primary) {
        self->priv->primary = g_object_ref (primary);
        mm_port_serial_at_add_unsolicited_msg_handler (primary, self->priv->eif_regex,
                                                       (MMPortSerialAtUnsolicitedMsgFn) eif_received,
                                                       self, NULL);
    }
}

static void
dial_3gpp (MMBroadbandBearer   *_self,
           MMBaseModem         *modem,
           MMPortSerialAt      *primary,
           guint                cid,
           GCancellable        *cancellable,
           GAsyncReadyCallback  callback,
           gpointer             user_data)
{
    MMBroadbandBearerMtkCcci *self = MM_BROADBAND_BEARER_MTK_CCCI (_self);
    GTask                    *task;
    Dial3gppContext          *ctx;

    g_assert (primary != NULL);
    g_assert (!self->priv->connect_pending);

    task = g_task_new (self, cancellable, callback, user_data);
    ctx = g_slice_new0 (Dial3gppContext);
    ctx->modem = g_object_ref (modem);
    ctx->primary = g_object_ref (primary);
    g_task_set_task_data (task, ctx, (GDestroyNotify) dial_3gpp_context_free);

    ifup_reset (self);
    self->priv->cid = cid;
    self->priv->connect_pending = task;
    eif_handler_set (self, primary);

    mm_base_modem_at_command_full (modem, MM_IFACE_PORT_AT (primary), "+EIFCONFIG=3", 5,
                                   FALSE, FALSE, NULL,
                                   (GAsyncReadyCallback) eifconfig_ready,
                                   g_object_ref (self));
}

/*****************************************************************************/
/* IP configuration (static IPv4 from +EIF, IPv6 by SLAAC) */

typedef struct {
    MMPortSerialAt   *primary;
    MMBearerIpFamily  ip_family;
} GetIpConfigContext;

static void
get_ip_config_context_free (GetIpConfigContext *ctx)
{
    g_clear_object (&ctx->primary);
    g_slice_free (GetIpConfigContext, ctx);
}

static gboolean
get_ip_config_3gpp_finish (MMBroadbandBearer  *self,
                           GAsyncResult       *res,
                           MMBearerIpConfig  **ipv4_config,
                           MMBearerIpConfig  **ipv6_config,
                           GError            **error)
{
    MMBearerConnectResult *result;
    MMBearerIpConfig      *ipv4, *ipv6;

    result = g_task_propagate_pointer (G_TASK (res), error);
    if (!result)
        return FALSE;
    ipv4 = mm_bearer_connect_result_peek_ipv4_config (result);
    ipv6 = mm_bearer_connect_result_peek_ipv6_config (result);
    if (ipv4_config && ipv4)
        *ipv4_config = g_object_ref (ipv4);
    if (ipv6_config && ipv6)
        *ipv6_config = g_object_ref (ipv6);
    mm_bearer_connect_result_unref (result);
    return TRUE;
}

static void
cgcontrdp_ready (MMBaseModem  *modem,
                 GAsyncResult *res,
                 GTask        *task)
{
    MMBroadbandBearerMtkCcci *self = g_task_get_source_object (task);
    GetIpConfigContext       *ctx = g_task_get_task_data (task);
    g_autoptr(GError)         error = NULL;
    g_autoptr(GPtrArray)      dns4 = g_ptr_array_new_with_free_func (g_free);
    g_autoptr(GPtrArray)      dns6 = g_ptr_array_new_with_free_func (g_free);
    g_autoptr(MMBearerIpConfig) ipv4 = NULL;
    g_autoptr(MMBearerIpConfig) ipv6 = NULL;
    const gchar              *response;
    gboolean                  want4, want6;

    response = mm_base_modem_at_command_full_finish (modem, res, &error);
    if (response)
        parse_cgcontrdp_dns (response, dns4, dns6);
    else
        mm_obj_warn (self, "couldn't load DNS servers: %s", error->message);
    g_ptr_array_add (dns4, NULL);
    g_ptr_array_add (dns6, NULL);

    want4 = ctx->ip_family & (MM_BEARER_IP_FAMILY_IPV4 | MM_BEARER_IP_FAMILY_IPV4V6 | MM_BEARER_IP_FAMILY_ANY);
    want6 = ctx->ip_family & (MM_BEARER_IP_FAMILY_IPV6 | MM_BEARER_IP_FAMILY_IPV4V6 | MM_BEARER_IP_FAMILY_ANY);

    if (want4 && self->priv->ipv4) {
        ipv4 = mm_bearer_ip_config_new ();
        mm_bearer_ip_config_set_method (ipv4, MM_BEARER_IP_METHOD_STATIC);
        mm_bearer_ip_config_set_address (ipv4, self->priv->ipv4);
        mm_bearer_ip_config_set_prefix (ipv4, 32);
        if (dns4->len > 1)
            mm_bearer_ip_config_set_dns (ipv4, (const gchar **) dns4->pdata);
        if (self->priv->mtu)
            mm_bearer_ip_config_set_mtu (ipv4, self->priv->mtu);
    }
    if (want6 && self->priv->ipv6) {
        /* "DHCP" = automatic: the link-local address carries the
         * interface identifier for SLAAC */
        ipv6 = mm_bearer_ip_config_new ();
        mm_bearer_ip_config_set_method (ipv6, MM_BEARER_IP_METHOD_DHCP);
        mm_bearer_ip_config_set_address (ipv6, self->priv->ipv6);
        mm_bearer_ip_config_set_prefix (ipv6, 64);
        if (dns6->len > 1)
            mm_bearer_ip_config_set_dns (ipv6, (const gchar **) dns6->pdata);
        if (self->priv->mtu)
            mm_bearer_ip_config_set_mtu (ipv6, self->priv->mtu);
    }

    if (!ipv4 && !ipv6) {
        g_task_return_new_error (task, MM_CORE_ERROR, MM_CORE_ERROR_FAILED,
                                 "modem reported no address of the requested family");
        g_object_unref (task);
        return;
    }

    g_task_return_pointer (task,
                           mm_bearer_connect_result_new (MM_PORT (ctx->primary), ipv4, ipv6),
                           (GDestroyNotify) mm_bearer_connect_result_unref);
    g_object_unref (task);
}

static void
get_ip_config_3gpp (MMBroadbandBearer   *_self,
                    MMBroadbandModem    *modem,
                    MMPortSerialAt      *primary,
                    MMPortSerialAt      *secondary,
                    MMPort              *data,
                    guint                cid,
                    MMBearerIpFamily     ip_family,
                    GAsyncReadyCallback  callback,
                    gpointer             user_data)
{
    MMBroadbandBearerMtkCcci *self = MM_BROADBAND_BEARER_MTK_CCCI (_self);
    GetIpConfigContext       *ctx;
    GTask                    *task;
    g_autofree gchar         *cmd = NULL;

    task = g_task_new (self, NULL, callback, user_data);
    ctx = g_slice_new0 (GetIpConfigContext);
    ctx->primary = g_object_ref (primary);
    ctx->ip_family = ip_family;
    g_task_set_task_data (task, ctx, (GDestroyNotify) get_ip_config_context_free);

    /* the context actually activated, which may be the attach one (0) */
    cmd = g_strdup_printf ("+CGCONTRDP=%u", self->priv->cid);
    mm_base_modem_at_command_full (MM_BASE_MODEM (modem), MM_IFACE_PORT_AT (primary), cmd, 10,
                                   FALSE, FALSE, NULL,
                                   (GAsyncReadyCallback) cgcontrdp_ready,
                                   task);
}

/*****************************************************************************/
/* Disconnection */

static gboolean
disconnect_3gpp_finish (MMBroadbandBearer  *self,
                        GAsyncResult       *res,
                        GError            **error)
{
    return g_task_propagate_boolean (G_TASK (res), error);
}

static void
deactivate_ready (MMBaseModem  *modem,
                  GAsyncResult *res,
                  GTask        *task)
{
    MMBroadbandBearerMtkCcci *self = g_task_get_source_object (task);
    GError                   *error = NULL;

    self->priv->disconnecting = FALSE;
    ifup_reset (self);
    if (!mm_base_modem_at_command_full_finish (modem, res, &error))
        g_task_return_error (task, error);
    else
        g_task_return_boolean (task, TRUE);
    g_object_unref (task);
}

static void
disconnect_3gpp (MMBroadbandBearer   *_self,
                 MMBroadbandModem    *modem,
                 MMPortSerialAt      *primary,
                 MMPortSerialAt      *secondary,
                 MMPort              *data,
                 guint                cid,
                 GAsyncReadyCallback  callback,
                 gpointer             user_data)
{
    MMBroadbandBearerMtkCcci *self = MM_BROADBAND_BEARER_MTK_CCCI (_self);
    GTask                    *task;
    g_autofree gchar         *cmd = NULL;

    task = g_task_new (self, NULL, callback, user_data);
    self->priv->disconnecting = TRUE;
    cmd = g_strdup_printf ("+CGACT=0,%u", self->priv->cid);
    mm_base_modem_at_command_full (MM_BASE_MODEM (modem), MM_IFACE_PORT_AT (primary), cmd,
                                   MM_BASE_BEARER_DEFAULT_DISCONNECTION_TIMEOUT,
                                   FALSE, FALSE, NULL,
                                   (GAsyncReadyCallback) deactivate_ready,
                                   task);
}

/*****************************************************************************/

MMBaseBearer *
mm_broadband_bearer_mtk_ccci_new_finish (GAsyncResult  *res,
                                         GError       **error)
{
    g_autoptr(GObject) source = NULL;
    GObject           *bearer;

    source = g_async_result_get_source_object (res);
    bearer = g_async_initable_new_finish (G_ASYNC_INITABLE (source), res, error);
    if (!bearer)
        return NULL;

    /* Only export valid bearers */
    mm_base_bearer_export (MM_BASE_BEARER (bearer));
    return MM_BASE_BEARER (bearer);
}

void
mm_broadband_bearer_mtk_ccci_new (MMBroadbandModem    *modem,
                                  MMBearerProperties  *config,
                                  GCancellable        *cancellable,
                                  GAsyncReadyCallback  callback,
                                  gpointer             user_data)
{
    g_async_initable_new_async (MM_TYPE_BROADBAND_BEARER_MTK_CCCI,
                                G_PRIORITY_DEFAULT,
                                cancellable,
                                callback,
                                user_data,
                                MM_BASE_BEARER_MODEM, modem,
                                MM_BIND_TO, modem,
                                MM_BASE_BEARER_CONFIG, config,
                                NULL);
}

static void
mm_broadband_bearer_mtk_ccci_init (MMBroadbandBearerMtkCcci *self)
{
    self->priv = mm_broadband_bearer_mtk_ccci_get_instance_private (self);
    self->priv->eif_id = -1;
    self->priv->eif_regex = g_regex_new (EIF_REGEX, G_REGEX_RAW | G_REGEX_OPTIMIZE, 0, NULL);
    g_assert (self->priv->eif_regex);
}

static void
dispose (GObject *object)
{
    MMBroadbandBearerMtkCcci *self = MM_BROADBAND_BEARER_MTK_CCCI (object);

    if (self->priv->connect_timeout_id) {
        g_source_remove (self->priv->connect_timeout_id);
        self->priv->connect_timeout_id = 0;
    }
    eif_handler_set (self, NULL);
    G_OBJECT_CLASS (mm_broadband_bearer_mtk_ccci_parent_class)->dispose (object);
}

static void
finalize (GObject *object)
{
    MMBroadbandBearerMtkCcci *self = MM_BROADBAND_BEARER_MTK_CCCI (object);

    ifup_reset (self);
    g_regex_unref (self->priv->eif_regex);
    G_OBJECT_CLASS (mm_broadband_bearer_mtk_ccci_parent_class)->finalize (object);
}

static void
mm_broadband_bearer_mtk_ccci_class_init (MMBroadbandBearerMtkCcciClass *klass)
{
    GObjectClass           *object_class = G_OBJECT_CLASS (klass);
    MMBaseBearerClass      *base_bearer_class = MM_BASE_BEARER_CLASS (klass);
    MMBroadbandBearerClass *broadband_bearer_class = MM_BROADBAND_BEARER_CLASS (klass);

    object_class->dispose = dispose;
    object_class->finalize = finalize;

    /* +CGACT? does not tell which context carries the data; the modem
     * reports a lost interface with +EIF ifdown instead */
    base_bearer_class->load_connection_status = NULL;
    base_bearer_class->load_connection_status_finish = NULL;
#if defined WITH_SUSPEND_RESUME
    base_bearer_class->reload_connection_status = NULL;
    base_bearer_class->reload_connection_status_finish = NULL;
#endif

    broadband_bearer_class->dial_3gpp = dial_3gpp;
    broadband_bearer_class->dial_3gpp_finish = dial_3gpp_finish;
    broadband_bearer_class->get_ip_config_3gpp = get_ip_config_3gpp;
    broadband_bearer_class->get_ip_config_3gpp_finish = get_ip_config_3gpp_finish;
    broadband_bearer_class->disconnect_3gpp = disconnect_3gpp;
    broadband_bearer_class->disconnect_3gpp_finish = disconnect_3gpp_finish;
}
