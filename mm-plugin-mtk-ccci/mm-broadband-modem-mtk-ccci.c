/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * MediaTek modem behind the mainline CCCI driver: a generic AT modem whose
 * data bearers need the +EIF handshake (see mm-broadband-bearer-mtk-ccci.c)
 * and whose incoming calls must be approved over MIPC (see
 * mm-mtk-ccci-mipc.c).
 *
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#include <config.h>

#include <ModemManager.h>
#define _LIBMM_INSIDE_MM
#include <libmm-glib.h>

#include "mm-broadband-modem-mtk-ccci.h"
#include "mm-broadband-bearer-mtk-ccci.h"
#include "mm-base-modem-at.h"
#include "mm-call-at.h"
#include "mm-iface-modem.h"
#include "mm-iface-modem-3gpp-profile-manager.h"
#include "mm-iface-modem-voice.h"
#include "mm-log-object.h"
#include "mm-modem-helpers.h"
#include "mm-mtk-ccci-mipc.h"

/* the MIPC channel the stock RIL does not use for its own session */
#define MIPC_CALL_PORT "/dev/ccci_mipc0"

static void iface_modem_init                      (MMIfaceModemInterface                   *iface);
static void iface_modem_3gpp_profile_manager_init (MMIfaceModem3gppProfileManagerInterface *iface);
static void iface_modem_voice_init                (MMIfaceModemVoiceInterface              *iface);

static MMIfaceModem3gppProfileManagerInterface *iface_modem_3gpp_profile_manager_parent;
static MMIfaceModemVoiceInterface              *iface_modem_voice_parent;

G_DEFINE_TYPE_EXTENDED (MMBroadbandModemMtkCcci, mm_broadband_modem_mtk_ccci, MM_TYPE_BROADBAND_MODEM, 0,
                        G_IMPLEMENT_INTERFACE (MM_TYPE_IFACE_MODEM, iface_modem_init)
                        G_IMPLEMENT_INTERFACE (MM_TYPE_IFACE_MODEM_3GPP_PROFILE_MANAGER, iface_modem_3gpp_profile_manager_init)
                        G_IMPLEMENT_INTERFACE (MM_TYPE_IFACE_MODEM_VOICE, iface_modem_voice_init))

/*****************************************************************************/
/* Profile format (3GPP profile manager interface)
 *
 * Context 0 is the PDN of the LTE attach: activating it only works when
 * the attach set it up, so never pick it for a new connection. The bearer
 * falls back to it when the network refuses a second context to the same
 * APN. */

static gboolean
profile_manager_check_format_finish (MMIfaceModem3gppProfileManager  *self,
                                     GAsyncResult                    *res,
                                     gboolean                        *new_id,
                                     gint                            *min_profile_id,
                                     gint                            *max_profile_id,
                                     GEqualFunc                      *apn_cmp,
                                     MM3gppProfileCmpFlags           *profile_cmp_flags,
                                     GError                         **error)
{
    if (!iface_modem_3gpp_profile_manager_parent->check_format_finish (self, res, new_id,
                                                                      min_profile_id, max_profile_id,
                                                                      apn_cmp, profile_cmp_flags,
                                                                      error))
        return FALSE;
    if (min_profile_id && *min_profile_id < 1)
        *min_profile_id = 1;
    return TRUE;
}

/*****************************************************************************/
/* Create bearer (Modem interface) */

static MMBaseBearer *
modem_create_bearer_finish (MMIfaceModem  *self,
                            GAsyncResult  *res,
                            GError       **error)
{
    return g_task_propagate_pointer (G_TASK (res), error);
}

static void
bearer_new_ready (GObject      *source,
                  GAsyncResult *res,
                  GTask        *task)
{
    MMBaseBearer *bearer;
    GError       *error = NULL;

    bearer = mm_broadband_bearer_mtk_ccci_new_finish (res, &error);
    if (!bearer)
        g_task_return_error (task, error);
    else
        g_task_return_pointer (task, bearer, g_object_unref);
    g_object_unref (task);
}

static void
modem_create_bearer (MMIfaceModem        *self,
                     MMBearerProperties  *props,
                     GAsyncReadyCallback  callback,
                     gpointer             user_data)
{
    mm_obj_dbg (self, "creating MTK CCCI bearer...");
    mm_broadband_bearer_mtk_ccci_new (MM_BROADBAND_MODEM (self),
                                      props,
                                      NULL, /* cancellable */
                                      (GAsyncReadyCallback) bearer_new_ready,
                                      g_task_new (self, NULL, callback, user_data));
}

/*****************************************************************************/
/* Check support (Voice interface)
 *
 * The modem answers the "AT+CLCC=?" probe of the generic code with
 * +CME ERROR: 100 although +CLCC works. Without +CLCC the generic code
 * drops calls that do not keep sending RING (and this modem never does,
 * see mm-mtk-ccci-mipc.c) after 10 s and does not follow the call state, so
 * probe the plain command instead. */

static gboolean
modem_voice_check_support_finish (MMIfaceModemVoice  *self,
                                  GAsyncResult       *res,
                                  GError            **error)
{
    return g_task_propagate_boolean (G_TASK (res), error);
}

static void
clcc_check_ready (MMBaseModem  *self,
                  GAsyncResult *res,
                  GTask        *task)
{
    MMBroadbandModemMtkCcci *modem = MM_BROADBAND_MODEM_MTK_CCCI (self);
    g_autoptr(GError)        error = NULL;

    /* while the SIM is still locked, +CLCC fails with a SIM error */
    modem->clcc_supported = !!mm_base_modem_at_command_finish (self, res, &error) ||
        g_error_matches (error, MM_MOBILE_EQUIPMENT_ERROR, MM_MOBILE_EQUIPMENT_ERROR_SIM_PIN) ||
        g_error_matches (error, MM_MOBILE_EQUIPMENT_ERROR, MM_MOBILE_EQUIPMENT_ERROR_SIM_PUK) ||
        g_error_matches (error, MM_MOBILE_EQUIPMENT_ERROR, MM_MOBILE_EQUIPMENT_ERROR_SIM_BUSY);
    mm_obj_dbg (self, "+CLCC %ssupported", modem->clcc_supported ? "" : "not ");
    g_object_set (self,
                  MM_IFACE_MODEM_VOICE_PERIODIC_CALL_LIST_CHECK_DISABLED, !modem->clcc_supported,
                  NULL);
    g_task_return_boolean (task, TRUE);
    g_object_unref (task);
}

static void
parent_voice_check_support_ready (MMIfaceModemVoice *self,
                                  GAsyncResult      *res,
                                  GTask             *task)
{
    GError *error = NULL;

    if (!iface_modem_voice_parent->check_support_finish (self, res, &error)) {
        g_task_return_error (task, error);
        g_object_unref (task);
        return;
    }
    mm_base_modem_at_command (MM_BASE_MODEM (self),
                              "+CLCC",
                              5,
                              FALSE,
                              (GAsyncReadyCallback) clcc_check_ready,
                              task);
}

static void
modem_voice_check_support (MMIfaceModemVoice   *self,
                           GAsyncReadyCallback  callback,
                           gpointer             user_data)
{
    iface_modem_voice_parent->check_support (
        self,
        (GAsyncReadyCallback) parent_voice_check_support_ready,
        g_task_new (self, NULL, callback, user_data));
}

/*****************************************************************************/
/* Load call list (Voice interface)
 *
 * The modem lists international numbers without the leading '+' and only
 * says so with <type> 145 (as 3GPP TS 27.007 allows), which the generic
 * parser ignores: the number in the call log then cannot be dialled back.
 * Add the '+' for those. */

#define TOA_INTERNATIONAL 145

static gboolean
modem_voice_load_call_list_finish (MMIfaceModemVoice  *self,
                                   GAsyncResult       *res,
                                   GList             **out_call_info_list,
                                   GError            **error)
{
    GError *inner_error = NULL;
    GList  *list;

    list = g_task_propagate_pointer (G_TASK (res), &inner_error);
    if (inner_error) {
        g_propagate_error (error, inner_error);
        return FALSE;
    }
    *out_call_info_list = list;
    return TRUE;
}

static void
add_international_prefix (const gchar *response,
                          GList       *list)
{
    g_autoptr(GRegex)     r = NULL;
    g_autoptr(GMatchInfo) match_info = NULL;

    r = g_regex_new ("\\+CLCC:\\s*(\\d+),[^,]*,[^,]*,[^,]*,[^,]*,\\s*\"([^\"]*)\",\\s*(\\d+)",
                     G_REGEX_RAW, 0, NULL);
    g_assert (r);
    g_regex_match (r, response, 0, &match_info);
    for (; g_match_info_matches (match_info); g_match_info_next (match_info, NULL)) {
        guint  idx = 0, type = 0;
        GList *l;

        if (!mm_get_uint_from_match_info (match_info, 1, &idx) ||
            !mm_get_uint_from_match_info (match_info, 3, &type) ||
            type != TOA_INTERNATIONAL)
            continue;
        for (l = list; l; l = g_list_next (l)) {
            MMCallInfo *info = l->data;
            gchar      *number;

            if (info->index != idx || !info->number || !info->number[0] || info->number[0] == '+')
                continue;
            number = g_strconcat ("+", info->number, NULL);
            g_free (info->number);
            info->number = number;
        }
    }
}

static void
clcc_ready (MMBaseModem  *self,
            GAsyncResult *res,
            GTask        *task)
{
    const gchar *response;
    GError      *error = NULL;
    GList       *list = NULL;

    response = mm_base_modem_at_command_finish (self, res, &error);
    if (!response || !mm_3gpp_parse_clcc_response (response, self, &list, &error)) {
        g_task_return_error (task, error);
    } else {
        add_international_prefix (response, list);
        g_task_return_pointer (task, list, (GDestroyNotify) mm_3gpp_call_info_list_free);
    }
    g_object_unref (task);
}

static void
modem_voice_load_call_list (MMIfaceModemVoice   *self,
                            GAsyncReadyCallback  callback,
                            gpointer             user_data)
{
    mm_base_modem_at_command (MM_BASE_MODEM (self),
                              "+CLCC",
                              5,
                              FALSE,
                              (GAsyncReadyCallback) clcc_ready,
                              g_task_new (self, NULL, callback, user_data));
}

/*****************************************************************************/
/* Create call (Voice interface) */

static MMBaseCall *
modem_voice_create_call (MMIfaceModemVoice *self,
                         MMCallDirection    direction,
                         const gchar       *number,
                         const guint        dtmf_tone_duration)
{
    gboolean clcc = MM_BROADBAND_MODEM_MTK_CCCI (self)->clcc_supported;

    /* as the generic code does when its +CLCC probe succeeds */
    return mm_call_at_new (MM_BASE_MODEM (self),
                           G_OBJECT (self),
                           direction,
                           number,
                           dtmf_tone_duration,
                           clcc,  /* skip incoming timeout */
                           clcc,  /* dialing->ringing supported */
                           clcc); /* ringing->active supported */
}

/*****************************************************************************/
/* Setup/cleanup unsolicited events (Voice interface)
 *
 * Besides the generic AT handlers, approve incoming calls over MIPC: the
 * modem does not ring before that. */

static gboolean
modem_voice_unsolicited_events_finish (MMIfaceModemVoice  *self,
                                       GAsyncResult       *res,
                                       GError            **error)
{
    return g_task_propagate_boolean (G_TASK (res), error);
}

static void
parent_setup_unsolicited_events_ready (MMIfaceModemVoice *self,
                                       GAsyncResult      *res,
                                       GTask             *task)
{
    MMBroadbandModemMtkCcci *modem = MM_BROADBAND_MODEM_MTK_CCCI (self);
    GError                  *error = NULL;

    if (!iface_modem_voice_parent->setup_unsolicited_events_finish (self, res, &error)) {
        g_task_return_error (task, error);
        g_object_unref (task);
        return;
    }

    if (!modem->mipc) {
        modem->mipc = mm_mtk_ccci_mipc_new (self, MIPC_CALL_PORT, &error);
        if (!modem->mipc) {
            /* calls still work outgoing; incoming ones never ring */
            mm_obj_warn (self, "cannot approve incoming calls: %s", error->message);
            g_clear_error (&error);
        }
    }
    g_task_return_boolean (task, TRUE);
    g_object_unref (task);
}

static void
modem_voice_setup_unsolicited_events (MMIfaceModemVoice   *self,
                                      GAsyncReadyCallback  callback,
                                      gpointer             user_data)
{
    iface_modem_voice_parent->setup_unsolicited_events (
        self,
        (GAsyncReadyCallback) parent_setup_unsolicited_events_ready,
        g_task_new (self, NULL, callback, user_data));
}

static void
parent_cleanup_unsolicited_events_ready (MMIfaceModemVoice *self,
                                         GAsyncResult      *res,
                                         GTask             *task)
{
    GError *error = NULL;

    if (!iface_modem_voice_parent->cleanup_unsolicited_events_finish (self, res, &error))
        g_task_return_error (task, error);
    else
        g_task_return_boolean (task, TRUE);
    g_object_unref (task);
}

static void
modem_voice_cleanup_unsolicited_events (MMIfaceModemVoice   *self,
                                        GAsyncReadyCallback  callback,
                                        gpointer             user_data)
{
    MMBroadbandModemMtkCcci *modem = MM_BROADBAND_MODEM_MTK_CCCI (self);

    g_clear_pointer (&modem->mipc, mm_mtk_ccci_mipc_free);
    iface_modem_voice_parent->cleanup_unsolicited_events (
        self,
        (GAsyncReadyCallback) parent_cleanup_unsolicited_events_ready,
        g_task_new (self, NULL, callback, user_data));
}

/*****************************************************************************/
/* Ports */

static void
setup_ports (MMBroadbandModem *self)
{
    g_autoptr(GRegex)  eif = NULL;
    MMPortSerialAt    *primary;

    MM_BROADBAND_MODEM_CLASS (mm_broadband_modem_mtk_ccci_parent_class)->setup_ports (self);

    /* +EIF reports outside of a connection attempt: swallow them. A bearer
     * installs its own handler for the same pattern while it lives. */
    primary = mm_base_modem_peek_port_primary (MM_BASE_MODEM (self));
    if (!primary)
        return;
    eif = g_regex_new ("\\r\\n\\+EIF:\\s*(\\d+),\\s*([a-z]+)([^\\r\\n]*)\\r\\n",
                       G_REGEX_RAW | G_REGEX_OPTIMIZE, 0, NULL);
    mm_port_serial_at_add_unsolicited_msg_handler (primary, eif, NULL, NULL, NULL);
}

/*****************************************************************************/

MMBroadbandModemMtkCcci *
mm_broadband_modem_mtk_ccci_new (const gchar  *device,
                                 const gchar  *physdev,
                                 const gchar **drivers,
                                 const gchar  *plugin,
                                 guint16       vendor_id,
                                 guint16       product_id)
{
    return g_object_new (MM_TYPE_BROADBAND_MODEM_MTK_CCCI,
                         MM_BASE_MODEM_DEVICE, device,
                         MM_BASE_MODEM_PHYSDEV, physdev,
                         MM_BASE_MODEM_DRIVERS, drivers,
                         MM_BASE_MODEM_PLUGIN, plugin,
                         MM_BASE_MODEM_VENDOR_ID, vendor_id,
                         MM_BASE_MODEM_PRODUCT_ID, product_id,
                         /* data only over the ccmni net ports */
                         MM_BASE_MODEM_DATA_NET_SUPPORTED, TRUE,
                         MM_BASE_MODEM_DATA_TTY_SUPPORTED, FALSE,
                         NULL);
}

static void
mm_broadband_modem_mtk_ccci_init (MMBroadbandModemMtkCcci *self)
{
}

static void
finalize (GObject *object)
{
    MMBroadbandModemMtkCcci *self = MM_BROADBAND_MODEM_MTK_CCCI (object);

    g_clear_pointer (&self->mipc, mm_mtk_ccci_mipc_free);
    G_OBJECT_CLASS (mm_broadband_modem_mtk_ccci_parent_class)->finalize (object);
}

static void
iface_modem_init (MMIfaceModemInterface *iface)
{
    iface->create_bearer = modem_create_bearer;
    iface->create_bearer_finish = modem_create_bearer_finish;
}

static void
iface_modem_3gpp_profile_manager_init (MMIfaceModem3gppProfileManagerInterface *iface)
{
    iface_modem_3gpp_profile_manager_parent = g_type_interface_peek_parent (iface);

    iface->check_format = iface_modem_3gpp_profile_manager_parent->check_format;
    iface->check_format_finish = profile_manager_check_format_finish;
}

static void
iface_modem_voice_init (MMIfaceModemVoiceInterface *iface)
{
    iface_modem_voice_parent = g_type_interface_peek_parent (iface);

    iface->check_support = modem_voice_check_support;
    iface->check_support_finish = modem_voice_check_support_finish;
    iface->create_call = modem_voice_create_call;
    iface->load_call_list = modem_voice_load_call_list;
    iface->load_call_list_finish = modem_voice_load_call_list_finish;
    iface->setup_unsolicited_events = modem_voice_setup_unsolicited_events;
    iface->setup_unsolicited_events_finish = modem_voice_unsolicited_events_finish;
    iface->cleanup_unsolicited_events = modem_voice_cleanup_unsolicited_events;
    iface->cleanup_unsolicited_events_finish = modem_voice_unsolicited_events_finish;
}

static void
mm_broadband_modem_mtk_ccci_class_init (MMBroadbandModemMtkCcciClass *klass)
{
    GObjectClass          *object_class = G_OBJECT_CLASS (klass);
    MMBroadbandModemClass *broadband_modem_class = MM_BROADBAND_MODEM_CLASS (klass);

    object_class->finalize = finalize;
    broadband_modem_class->setup_ports = setup_ports;
}
