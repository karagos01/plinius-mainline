/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Plugin for MediaTek modems behind the mainline CCCI driver (mtk-ccci,
 * e.g. the MT6878 of the Volla Phone Plinius): the AT channel is a WWAN
 * port (wwan0at0), data goes over the ccmniN net devices.
 *
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#include <string.h>
#include <gmodule.h>

#define _LIBMM_INSIDE_MM
#include <libmm-glib.h>

#include "mm-plugin-common.h"
#include "mm-broadband-modem-mtk-ccci.h"

#define MM_TYPE_PLUGIN_MTK_CCCI mm_plugin_mtk_ccci_get_type ()
MM_DEFINE_PLUGIN (MTK_CCCI, mtk_ccci, MtkCcci)

static MMBaseModem *
create_modem (MMPlugin     *self,
              const gchar  *uid,
              const gchar  *physdev,
              const gchar **drivers,
              guint16       vendor,
              guint16       product,
              guint16       subsystem_vendor,
              guint16       subsystem_device,
              GList        *probes,
              GError      **error)
{
    return MM_BASE_MODEM (mm_broadband_modem_mtk_ccci_new (uid,
                                                           physdev,
                                                           drivers,
                                                           mm_plugin_get_name (self),
                                                           vendor,
                                                           product));
}

MM_PLUGIN_NAMED_CREATOR_SCOPE MMPlugin *
mm_plugin_create_mtk_ccci (void)
{
    static const gchar *subsystems[] = { "wwan", "net", NULL };
    static const gchar *drivers[] = { "mtk-ccci", NULL };

    return MM_PLUGIN (g_object_new (MM_TYPE_PLUGIN_MTK_CCCI,
                                    MM_PLUGIN_NAME,               MM_MODULE_NAME,
                                    MM_PLUGIN_ALLOWED_SUBSYSTEMS, subsystems,
                                    MM_PLUGIN_ALLOWED_DRIVERS,    drivers,
                                    MM_PLUGIN_ALLOWED_AT,         TRUE,
                                    NULL));
}

static void
mm_plugin_mtk_ccci_init (MMPluginMtkCcci *self)
{
}

static void
mm_plugin_mtk_ccci_class_init (MMPluginMtkCcciClass *klass)
{
    MMPluginClass *plugin_class = MM_PLUGIN_CLASS (klass);

    plugin_class->create_modem = create_modem;
}
