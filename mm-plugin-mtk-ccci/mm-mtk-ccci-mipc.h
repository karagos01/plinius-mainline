/* -*- Mode: C; tab-width: 4; indent-tabs-mode: nil; c-basic-offset: 4 -*- */
/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * Incoming call approval over MIPC for MediaTek CCCI modems.
 *
 * Copyright (C) 2026 Jan Benedikt <karagos01@gmail.com>
 */

#ifndef MM_MTK_CCCI_MIPC_H
#define MM_MTK_CCCI_MIPC_H

#include <glib.h>

#include "mm-iface-modem-voice.h"

typedef struct _MMMtkCcciMipc MMMtkCcciMipc;

MMMtkCcciMipc *mm_mtk_ccci_mipc_new  (MMIfaceModemVoice  *modem,
                                      const gchar        *path,
                                      GError            **error);
void           mm_mtk_ccci_mipc_free (MMMtkCcciMipc      *self);

#endif /* MM_MTK_CCCI_MIPC_H */
