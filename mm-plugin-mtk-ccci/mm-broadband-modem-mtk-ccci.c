/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * MediaTek modem behind the mainline CCCI driver: a generic AT modem whose
 * data bearers need the +EIF handshake (see mm-broadband-bearer-mtk-ccci.c).
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
#include "mm-iface-modem.h"
#include "mm-iface-modem-3gpp-profile-manager.h"
#include "mm-log-object.h"

static void iface_modem_init                      (MMIfaceModemInterface                   *iface);
static void iface_modem_3gpp_profile_manager_init (MMIfaceModem3gppProfileManagerInterface *iface);

static MMIfaceModem3gppProfileManagerInterface *iface_modem_3gpp_profile_manager_parent;

G_DEFINE_TYPE_EXTENDED (MMBroadbandModemMtkCcci, mm_broadband_modem_mtk_ccci, MM_TYPE_BROADBAND_MODEM, 0,
                        G_IMPLEMENT_INTERFACE (MM_TYPE_IFACE_MODEM, iface_modem_init)
                        G_IMPLEMENT_INTERFACE (MM_TYPE_IFACE_MODEM_3GPP_PROFILE_MANAGER, iface_modem_3gpp_profile_manager_init))

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
mm_broadband_modem_mtk_ccci_class_init (MMBroadbandModemMtkCcciClass *klass)
{
    MMBroadbandModemClass *broadband_modem_class = MM_BROADBAND_MODEM_CLASS (klass);

    broadband_modem_class->setup_ports = setup_ports;
}
