/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#ifndef MM_BROADBAND_BEARER_MTK_CCCI_H
#define MM_BROADBAND_BEARER_MTK_CCCI_H

#include <glib.h>
#include <glib-object.h>

#define _LIBMM_INSIDE_MM
#include <libmm-glib.h>

#include "mm-broadband-bearer.h"
#include "mm-broadband-modem.h"

#define MM_TYPE_BROADBAND_BEARER_MTK_CCCI            (mm_broadband_bearer_mtk_ccci_get_type ())
#define MM_BROADBAND_BEARER_MTK_CCCI(obj)            (G_TYPE_CHECK_INSTANCE_CAST ((obj), MM_TYPE_BROADBAND_BEARER_MTK_CCCI, MMBroadbandBearerMtkCcci))
#define MM_BROADBAND_BEARER_MTK_CCCI_CLASS(klass)    (G_TYPE_CHECK_CLASS_CAST ((klass),  MM_TYPE_BROADBAND_BEARER_MTK_CCCI, MMBroadbandBearerMtkCcciClass))
#define MM_IS_BROADBAND_BEARER_MTK_CCCI(obj)         (G_TYPE_CHECK_INSTANCE_TYPE ((obj), MM_TYPE_BROADBAND_BEARER_MTK_CCCI))
#define MM_IS_BROADBAND_BEARER_MTK_CCCI_CLASS(klass) (G_TYPE_CHECK_CLASS_TYPE ((klass),  MM_TYPE_BROADBAND_BEARER_MTK_CCCI))
#define MM_BROADBAND_BEARER_MTK_CCCI_GET_CLASS(obj)  (G_TYPE_INSTANCE_GET_CLASS ((obj),  MM_TYPE_BROADBAND_BEARER_MTK_CCCI, MMBroadbandBearerMtkCcciClass))

typedef struct _MMBroadbandBearerMtkCcci MMBroadbandBearerMtkCcci;
typedef struct _MMBroadbandBearerMtkCcciClass MMBroadbandBearerMtkCcciClass;
typedef struct _MMBroadbandBearerMtkCcciPrivate MMBroadbandBearerMtkCcciPrivate;

struct _MMBroadbandBearerMtkCcci {
    MMBroadbandBearer parent;
    MMBroadbandBearerMtkCcciPrivate *priv;
};

struct _MMBroadbandBearerMtkCcciClass {
    MMBroadbandBearerClass parent;
};

GType mm_broadband_bearer_mtk_ccci_get_type (void);
G_DEFINE_AUTOPTR_CLEANUP_FUNC (MMBroadbandBearerMtkCcci, g_object_unref)

void          mm_broadband_bearer_mtk_ccci_new        (MMBroadbandModem     *modem,
                                                       MMBearerProperties   *config,
                                                       GCancellable         *cancellable,
                                                       GAsyncReadyCallback   callback,
                                                       gpointer              user_data);
MMBaseBearer *mm_broadband_bearer_mtk_ccci_new_finish (GAsyncResult         *res,
                                                       GError              **error);

#endif /* MM_BROADBAND_BEARER_MTK_CCCI_H */
