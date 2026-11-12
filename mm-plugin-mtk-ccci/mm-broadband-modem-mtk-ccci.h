/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#ifndef MM_BROADBAND_MODEM_MTK_CCCI_H
#define MM_BROADBAND_MODEM_MTK_CCCI_H

#include "mm-broadband-modem.h"

#define MM_TYPE_BROADBAND_MODEM_MTK_CCCI            (mm_broadband_modem_mtk_ccci_get_type ())
#define MM_BROADBAND_MODEM_MTK_CCCI(obj)            (G_TYPE_CHECK_INSTANCE_CAST ((obj), MM_TYPE_BROADBAND_MODEM_MTK_CCCI, MMBroadbandModemMtkCcci))
#define MM_BROADBAND_MODEM_MTK_CCCI_CLASS(klass)    (G_TYPE_CHECK_CLASS_CAST ((klass),  MM_TYPE_BROADBAND_MODEM_MTK_CCCI, MMBroadbandModemMtkCcciClass))
#define MM_IS_BROADBAND_MODEM_MTK_CCCI(obj)         (G_TYPE_CHECK_INSTANCE_TYPE ((obj), MM_TYPE_BROADBAND_MODEM_MTK_CCCI))
#define MM_IS_BROADBAND_MODEM_MTK_CCCI_CLASS(klass) (G_TYPE_CHECK_CLASS_TYPE ((klass),  MM_TYPE_BROADBAND_MODEM_MTK_CCCI))
#define MM_BROADBAND_MODEM_MTK_CCCI_GET_CLASS(obj)  (G_TYPE_INSTANCE_GET_CLASS ((obj),  MM_TYPE_BROADBAND_MODEM_MTK_CCCI, MMBroadbandModemMtkCcciClass))

typedef struct _MMBroadbandModemMtkCcci MMBroadbandModemMtkCcci;
typedef struct _MMBroadbandModemMtkCcciClass MMBroadbandModemMtkCcciClass;

struct _MMBroadbandModemMtkCcci {
    MMBroadbandModem      parent;
    struct _MMMtkCcciMipc *mipc; /* incoming call approval */
    gboolean               clcc_supported;
};

struct _MMBroadbandModemMtkCcciClass {
    MMBroadbandModemClass parent;
};

GType mm_broadband_modem_mtk_ccci_get_type (void);

MMBroadbandModemMtkCcci *mm_broadband_modem_mtk_ccci_new (const gchar  *device,
                                                          const gchar  *physdev,
                                                          const gchar **drivers,
                                                          const gchar  *plugin,
                                                          guint16       vendor_id,
                                                          guint16       product_id);

#endif /* MM_BROADBAND_MODEM_MTK_CCCI_H */
