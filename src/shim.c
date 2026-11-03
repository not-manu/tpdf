#include <mupdf/fitz.h>
#include <stdio.h>
#include <stdlib.h>

#define CACHED_LISTS 6
#define MAX_HITS 128

typedef struct {
    int page;
    fz_display_list *list;
    fz_rect bounds;
    unsigned long used;
} cached_list;

typedef struct {
    fz_context *ctx;
    fz_document *doc;
    int pages;
    unsigned long tick;
    cached_list lists[CACHED_LISTS];
} tpdf;

tpdf *tpdf_open(const char *path, size_t store, char *error, size_t error_size)
{
    fz_context *ctx = fz_new_context(NULL, NULL, store);
    if (!ctx) {
        snprintf(error, error_size, "mupdf was upgraded, rebuild tpdf");
        return NULL;
    }
    fz_set_error_callback(ctx, NULL, NULL);
    fz_set_warning_callback(ctx, NULL, NULL);

    tpdf *t = calloc(1, sizeof *t);
    if (!t) {
        snprintf(error, error_size, "out of memory");
        fz_drop_context(ctx);
        return NULL;
    }
    t->ctx = ctx;

    fz_try(ctx) {
        fz_register_document_handlers(ctx);
        t->doc = fz_open_document(ctx, path);
        if (fz_needs_password(ctx, t->doc))
            fz_throw(ctx, FZ_ERROR_GENERIC, "password-protected files are not supported");
        t->pages = fz_count_pages(ctx, t->doc);
    }
    fz_catch(ctx) {
        snprintf(error, error_size, "%s", fz_caught_message(ctx));
        fz_ignore_error(ctx);
        fz_drop_document(ctx, t->doc);
        fz_drop_context(ctx);
        free(t);
        return NULL;
    }
    return t;
}

void tpdf_close(tpdf *t)
{
    for (int i = 0; i < CACHED_LISTS; i++)
        fz_drop_display_list(t->ctx, t->lists[i].list);
    fz_drop_document(t->ctx, t->doc);
    fz_drop_context(t->ctx);
    free(t);
}

int tpdf_pages(tpdf *t)
{
    return t->pages;
}

int tpdf_bounds(tpdf *t, int index, float *width, float *height)
{
    fz_context *ctx = t->ctx;
    fz_page *page = NULL;
    int ok = 1;
    fz_var(page);
    fz_var(ok);
    fz_try(ctx) {
        page = fz_load_page(ctx, t->doc, index);
        fz_rect bounds = fz_bound_page(ctx, page);
        *width = bounds.x1 - bounds.x0;
        *height = bounds.y1 - bounds.y0;
    }
    fz_always(ctx)
        fz_drop_page(ctx, page);
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        ok = 0;
    }
    return ok;
}

static cached_list *load_list(tpdf *t, int index)
{
    fz_context *ctx = t->ctx;
    cached_list *slot = &t->lists[0];
    for (int i = 0; i < CACHED_LISTS; i++) {
        cached_list *entry = &t->lists[i];
        if (entry->list && entry->page == index) {
            entry->used = ++t->tick;
            return entry;
        }
        if (entry->used < slot->used)
            slot = entry;
    }

    fz_page *page = NULL;
    fz_display_list *list = NULL;
    fz_rect bounds = fz_empty_rect;
    fz_var(page);
    fz_var(list);
    fz_var(bounds);
    fz_try(ctx) {
        page = fz_load_page(ctx, t->doc, index);
        bounds = fz_bound_page(ctx, page);
        list = fz_new_display_list_from_page(ctx, page);
    }
    fz_always(ctx)
        fz_drop_page(ctx, page);
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        fz_drop_display_list(ctx, list);
        return NULL;
    }

    fz_drop_display_list(ctx, slot->list);
    slot->page = index;
    slot->list = list;
    slot->bounds = bounds;
    slot->used = ++t->tick;
    return slot;
}

int tpdf_render(tpdf *t, int index, float scale, int x, int y, int width, int height,
                unsigned char *rgb, int dark, int black, int white)
{
    fz_context *ctx = t->ctx;
    cached_list *entry = load_list(t, index);
    if (!entry)
        return 0;

    fz_pixmap *pixmap = NULL;
    fz_device *device = NULL;
    int ok = 1;
    fz_var(pixmap);
    fz_var(device);
    fz_var(ok);
    fz_try(ctx) {
        fz_irect area = fz_make_irect(x, y, x + width, y + height);
        pixmap = fz_new_pixmap_with_bbox_and_data(ctx, fz_device_rgb(ctx), area, NULL, 0, rgb);
        fz_clear_pixmap_with_value(ctx, pixmap, 0xff);
        fz_matrix ctm = fz_concat(fz_translate(-entry->bounds.x0, -entry->bounds.y0),
                                  fz_scale(scale, scale));
        device = fz_new_draw_device(ctx, fz_identity, pixmap);
        fz_run_display_list(ctx, entry->list, device, ctm, fz_rect_from_irect(area), NULL);
        fz_close_device(ctx, device);
        if (dark)
            fz_tint_pixmap(ctx, pixmap, black, white);
    }
    fz_always(ctx) {
        fz_drop_device(ctx, device);
        fz_drop_pixmap(ctx, pixmap);
    }
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        ok = 0;
    }
    return ok;
}

int tpdf_search(tpdf *t, int index, const char *needle, float *out, int max)
{
    fz_context *ctx = t->ctx;
    fz_page *page = NULL;
    fz_quad quads[MAX_HITS];
    fz_rect bounds = fz_empty_rect;
    int count = 0;
    if (max > MAX_HITS)
        max = MAX_HITS;
    fz_var(page);
    fz_var(count);
    fz_var(bounds);
    fz_try(ctx) {
        page = fz_load_page(ctx, t->doc, index);
        bounds = fz_bound_page(ctx, page);
        count = fz_search_page(ctx, page, needle, NULL, quads, max);
    }
    fz_always(ctx)
        fz_drop_page(ctx, page);
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        return 0;
    }
    for (int i = 0; i < count; i++) {
        fz_rect rect = fz_rect_from_quad(quads[i]);
        out[i * 4] = rect.x0 - bounds.x0;
        out[i * 4 + 1] = rect.y0 - bounds.y0;
        out[i * 4 + 2] = rect.x1 - bounds.x0;
        out[i * 4 + 3] = rect.y1 - bounds.y0;
    }
    return count;
}
