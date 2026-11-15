/*
 * t42render: draw a teletext page from a raw T42 dump (42-byte packets, as ttxd's
 * debug port sends them) with libzvbi, at a chosen presentation level.
 *
 *   t42render dump.t42 100 2.5 out.ppm     (level: 1, 1.5, 2.5 or 3.5)
 *
 * libzvbi is an independent decoder (it also sits behind FFmpeg and VLC), so this
 * checks what ttxd puts on the wire, not what we meant to send.
 *
 * Build: apt install libzvbi-dev && cc -O2 -o t42render t42render.c -lzvbi
 * The output is a PPM of 12x10 pixels per cell; scale it up about 1.44x taller than
 * wide to see it with the shape it has on a 4:3 picture.
 */
#include <libzvbi.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int seen;

static void on_page(vbi_event *ev, void *user)
{
	(void) user;
	if (ev->type == VBI_EVENT_TTX_PAGE)
		seen++;
}

int main(int argc, char **argv)
{
	if (argc != 5) {
		fprintf(stderr, "usage: %s dump.t42 PAGE LEVEL out.ppm\n", argv[0]);
		return 2;
	}
	FILE *f = fopen(argv[1], "rb");
	if (!f) { perror(argv[1]); return 1; }
	int pgno = (int) strtol(argv[2], NULL, 16);
	vbi_wst_level level = !strcmp(argv[3], "1") ? VBI_WST_LEVEL_1
		: !strcmp(argv[3], "1.5") ? VBI_WST_LEVEL_1p5
		: !strcmp(argv[3], "2.5") ? VBI_WST_LEVEL_2p5 : VBI_WST_LEVEL_3p5;

	vbi_decoder *vbi = vbi_decoder_new();
	/* Pages only reach the cache while someone listens for them. */
	if (!vbi_event_handler_register(vbi, VBI_EVENT_TTX_PAGE, on_page, NULL)) fprintf(stderr, "register failed\n");
	/* All packets in one call: vbi_decode treats a gap in the frame times as a
	   channel change and throws away half-received pages. */
	size_t cap = 1024, n = 0;
	vbi_sliced *s = malloc(cap * sizeof *s);
	unsigned char pk[42];
	while (fread(pk, 1, 42, f) == 42) {
		if (n == cap)
			s = realloc(s, (cap *= 2) * sizeof *s);
		s[n].id = VBI_SLICED_TELETEXT_B;
		s[n].line = 7 + n % 5;
		memcpy(s[n].data, pk, 42);
		n++;
	}
	vbi_decode(vbi, s, (int) n, 1.0);

	vbi_page pg;
	if (!vbi_fetch_vt_page(vbi, &pg, pgno, VBI_ANY_SUBNO, level, 25, TRUE)) {
		fprintf(stderr, "page %x not in the dump (%d pages decoded)\n", pgno, seen);
		return 1;
	}
	int w = pg.columns * 12, h = pg.rows * 10;
	uint32_t *buf = calloc((size_t) w * h, 4);
	vbi_draw_vt_page_region(&pg, VBI_PIXFMT_RGBA32_LE, buf, w * 4, 0, 0,
				pg.columns, pg.rows, 0, 1);
	FILE *o = fopen(argv[4], "wb");
	fprintf(o, "P6\n%d %d\n255\n", w, h);
	for (int i = 0; i < w * h; i++) {
		unsigned char rgb[3] = { buf[i] & 255, (buf[i] >> 8) & 255, (buf[i] >> 16) & 255 };
		fwrite(rgb, 1, 3, o);
	}
	fclose(o);
	vbi_unref_page(&pg);
	vbi_decoder_delete(vbi);
	return 0;
}
