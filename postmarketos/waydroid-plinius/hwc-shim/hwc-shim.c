// SPDX-License-Identifier: MIT
/*
 * hwcomposer.waydroid.so shim for Waydroid on postmarketOS.
 *
 * Phosh makes room for the on-screen keyboard by shrinking the windows,
 * and Waydroid's hwcomposer turns every window resize into a resize of
 * the whole Android display. Apps that don't handle size changes are
 * then relaunched, lose their text field focus and the keyboard goes
 * away again. The pmOS keyboard tells Android about the keyboard as an
 * input method inset instead, so a shrink that keeps the width is not
 * passed on: Android keeps the display size, and the part of it under
 * the keyboard is covered by the input method's (invisible) window.
 *
 * The shim loads Waydroid's hwcomposer (hwcomposer.waydroid-orig.so),
 * exports its module and replaces the configure handler of its
 * xdg_toplevel listener. The listener is static, so it is found at a
 * fixed offset that is only used when the build ID matches.
 *
 * Built against bionic without the NDK; it uses no headers.
 */

typedef unsigned long size_t;
typedef int int32_t;
typedef unsigned int uint32_t;
typedef unsigned short uint16_t;
typedef unsigned long uint64_t;

#define RTLD_NOW 2
#define PROT_READ 1
#define PROT_WRITE 2
#define ANDROID_LOG_INFO 4
#define ANDROID_LOG_WARN 5

void *dlopen(const char *, int);
void *dlsym(void *, const char *);
const char *dlerror(void);
typedef struct {
	const char *dli_fname;
	void *dli_fbase;
	const char *dli_sname;
	void *dli_saddr;
} Dl_info;
int dladdr(const void *, Dl_info *);
int mprotect(void *, size_t, int);
int memcmp(const void *, const void *, size_t);
void *memcpy(void *, const void *, size_t);
int __android_log_print(int, const char *, const char *, ...);

#define TAG "hwcomposer-shim"
#define ORIG "/vendor/lib64/hw/hwcomposer.waydroid-orig.so"

/* hw_module_t, LP64 */
struct hw_module {
	uint32_t tag;
	uint16_t module_api_version;
	uint16_t hal_api_version;
	const char *id;
	const char *name;
	const char *author;
	void *methods;
	void *dso;
	uint64_t reserved[32 - 7];
};

__attribute__((visibility("default"))) struct hw_module HMI;

/* lineage-20 Waydroid hwcomposer, as on the Plinius image */
static const struct {
	unsigned char build_id[16];
	size_t listener;	/* xdg_toplevel_listener */
	size_t configure;	/* xdg_toplevel_handle_configure */
	size_t close;		/* xdg_toplevel_handle_close */
} known[] = {
	{ { 0xc8, 0xb9, 0x65, 0xb6, 0x03, 0xd5, 0xac, 0xa4,
	    0xc3, 0x0b, 0xee, 0x7b, 0xaf, 0x8f, 0x69, 0xe7 },
	  0x6ea60, 0x42d64, 0x42eac },
};

typedef void (*configure_fn)(void *, void *, int32_t, int32_t, void *);
static configure_fn orig_configure;
static int32_t full_width, full_height;

static void configure(void *data, void *toplevel, int32_t width,
		      int32_t height, void *states)
{
	if (width > 1 && height > 1) {
		if (width == full_width && height < full_height)
			height = full_height;
		else {
			full_width = width;
			full_height = height;
		}
	}
	orig_configure(data, toplevel, width, height, states);
}

/* ELF64 */
struct ehdr {
	unsigned char ident[16];
	uint16_t type, machine;
	uint32_t version;
	uint64_t entry, phoff, shoff;
	uint32_t flags;
	uint16_t ehsize, phentsize, phnum, shentsize, shnum, shstrndx;
};
struct phdr {
	uint32_t type, flags;
	uint64_t offset, vaddr, paddr, filesz, memsz, align;
};
#define PT_LOAD 1
#define PT_NOTE 4

static int build_id_matches(const char *base, const unsigned char *id)
{
	const struct ehdr *eh = (const void *)base;
	const struct phdr *ph = (const void *)(base + eh->phoff);

	for (int i = 0; i < eh->phnum; i++) {
		if (ph[i].type != PT_NOTE)
			continue;
		const char *p = base + ph[i].vaddr, *end = p + ph[i].memsz;
		while (p + 12 <= end) {
			const uint32_t *n = (const void *)p;
			uint32_t namesz = n[0], descsz = n[1], type = n[2];
			const char *name = p + 12;
			const char *desc = name + ((namesz + 3) & ~3u);
			if (type == 3 && namesz == 4 && !memcmp(name, "GNU", 4))
				return descsz == 16 && !memcmp(desc, id, 16);
			p = desc + ((descsz + 3) & ~3u);
		}
	}
	return 0;
}

__attribute__((constructor)) static void init(void)
{
	void *h = dlopen(ORIG, RTLD_NOW);
	if (!h) {
		__android_log_print(ANDROID_LOG_WARN, TAG, "%s", dlerror());
		return;
	}
	const struct hw_module *mod = dlsym(h, "HMI");
	if (!mod) {
		__android_log_print(ANDROID_LOG_WARN, TAG, "%s", dlerror());
		return;
	}
	memcpy(&HMI, mod, sizeof(HMI));

	Dl_info info;
	if (!dladdr(mod, &info) || !info.dli_fbase)
		return;
	const char *base = info.dli_fbase;

	for (size_t i = 0; i < sizeof(known) / sizeof(known[0]); i++) {
		if (!build_id_matches(base, known[i].build_id))
			continue;
		void **listener = (void **)(base + known[i].listener);
		if (listener[0] != base + known[i].configure ||
		    listener[1] != base + known[i].close)
			break;
		void *page = (void *)((size_t)listener & ~0xfffUL);
		if (mprotect(page, 0x1000, PROT_READ | PROT_WRITE))
			break;
		orig_configure = (configure_fn)listener[0];
		listener[0] = (void *)configure;
		mprotect(page, 0x1000, PROT_READ);
		__android_log_print(ANDROID_LOG_INFO, TAG,
				    "keeping the display size on keyboard resizes");
		return;
	}
	__android_log_print(ANDROID_LOG_WARN, TAG,
			    "unknown hwcomposer build, resizes are passed on");
}
