#include <mupdf/fitz.h>
#include <mupdf/ucdn.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

#define CACHED_LISTS 6
#define MAX_HITS 128
#define MAX_DECOMPOSED 18

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

int tpdf_label(tpdf *t, int index, char *label, int size)
{
    fz_context *ctx = t->ctx;
    fz_page *page = NULL;
    int ok = 1;
    fz_var(page);
    fz_var(ok);
    fz_try(ctx) {
        page = fz_load_page(ctx, t->doc, index);
        fz_page_label(ctx, page, label, size);
    }
    fz_always(ctx)
        fz_drop_page(ctx, page);
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        ok = 0;
    }
    return ok;
}

void tpdf_link_at(tpdf *t, int index, float x0, float y0, float x1, float y1,
                  char **uri, int *target, float *target_y)
{
    fz_context *ctx = t->ctx;
    fz_page *page = NULL;
    fz_page *landing = NULL;
    fz_link *links = NULL;
    fz_var(page);
    fz_var(landing);
    fz_var(links);
    fz_try(ctx) {
        page = fz_load_page(ctx, t->doc, index);
        fz_rect bounds = fz_bound_page(ctx, page);
        fz_rect cell = fz_make_rect(x0 + bounds.x0, y0 + bounds.y0, x1 + bounds.x0, y1 + bounds.y0);
        links = fz_load_links(ctx, page);
        fz_link *hit = NULL;
        float most = 0;
        for (fz_link *link = links; link; link = link->next) {
            fz_rect overlap = fz_intersect_rect(link->rect, cell);
            float area = fz_is_empty_rect(overlap)
                ? 0
                : (overlap.x1 - overlap.x0) * (overlap.y1 - overlap.y0);
            if (area > most) {
                most = area;
                hit = link;
            }
        }
        if (hit && fz_is_external_link(ctx, hit->uri)) {
            *uri = fz_strdup(ctx, hit->uri);
        } else if (hit) {
            float y = NAN;
            fz_location location = fz_resolve_link(ctx, t->doc, hit->uri, NULL, &y);
            int number = fz_page_number_from_location(ctx, t->doc, location);
            if (number >= 0) {
                landing = fz_load_page(ctx, t->doc, number);
                *target_y = isfinite(y) ? y - fz_bound_page(ctx, landing).y0 : 0;
                *target = number;
            }
        }
    }
    fz_always(ctx) {
        fz_drop_link(ctx, links);
        fz_drop_page(ctx, landing);
        fz_drop_page(ctx, page);
    }
    fz_catch(ctx)
        fz_ignore_error(ctx);
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

char *tpdf_text(tpdf *t, int index, float x0, float y0, float x1, float y1)
{
    fz_context *ctx = t->ctx;
    fz_page *page = NULL;
    fz_stext_page *stext = NULL;
    char *text = NULL;
    fz_var(page);
    fz_var(stext);
    fz_var(text);
    fz_try(ctx) {
        fz_stext_options options = { 0 };
        options.flags = FZ_STEXT_DEHYPHENATE;
        page = fz_load_page(ctx, t->doc, index);
        fz_rect bounds = fz_bound_page(ctx, page);
        stext = fz_new_stext_page_from_page(ctx, page, &options);
        fz_rect area = fz_make_rect(x0 + bounds.x0, y0 + bounds.y0, x1 + bounds.x0, y1 + bounds.y0);
        text = fz_copy_rectangle(ctx, stext, area, 0);
    }
    fz_always(ctx) {
        fz_drop_stext_page(ctx, stext);
        fz_drop_page(ctx, page);
    }
    fz_catch(ctx) {
        fz_ignore_error(ctx);
        return NULL;
    }
    return text;
}

void tpdf_free_text(tpdf *t, char *text)
{
    fz_free(t->ctx, text);
}

static void fold_rune(fz_context *ctx, fz_buffer *out, int rune)
{
    uint32_t parts[MAX_DECOMPOSED];
    int count = ucdn_compat_decompose(rune, parts);
    if (count > 0) {
        for (int i = 0; i < count; i++)
            fold_rune(ctx, out, parts[i]);
        return;
    }
    if (fz_is_unicode_whitespace(rune) || fz_is_unicode_hyphen(rune))
        return;
    rune = fz_toupper(rune);
    if (ucdn_get_combining_class(rune) == 0)
        fz_append_rune(ctx, out, rune);
}

static char *fold_text(fz_context *ctx, const char *text)
{
    fz_buffer *out = fz_new_buffer(ctx, strlen(text) + 1);
    char *folded = NULL;
    fz_var(folded);
    fz_try(ctx) {
        while (*text) {
            int rune;
            text += fz_chartorune(&rune, text);
            fold_rune(ctx, out, rune);
        }
        fz_terminate_buffer(ctx, out);
        (void)fz_buffer_extract(ctx, out, (unsigned char **)&folded);
    }
    fz_always(ctx)
        fz_drop_buffer(ctx, out);
    fz_catch(ctx)
        fz_rethrow(ctx);
    return folded;
}

char *tpdf_fold(tpdf *t, const char *text)
{
    fz_context *ctx = t->ctx;
    char *folded = NULL;
    fz_var(folded);
    fz_try(ctx)
        folded = fold_text(ctx, text);
    fz_catch(ctx)
        fz_ignore_error(ctx);
    return folded;
}

char *tpdf_folded_page(tpdf *t, int index)
{
    fz_context *ctx = t->ctx;
    fz_buffer *flat = NULL;
    char *folded = NULL;
    fz_var(flat);
    fz_var(folded);
    fz_try(ctx) {
        fz_stext_options options = { 0 };
        options.flags = FZ_STEXT_DEHYPHENATE;
        flat = fz_new_buffer_from_flattened_page_number(ctx, t->doc, index, &options, FZ_TEXT_FLATTEN_ALL);
        folded = fold_text(ctx, fz_string_from_buffer(ctx, flat));
    }
    fz_always(ctx)
        fz_drop_buffer(ctx, flat);
    fz_catch(ctx)
        fz_ignore_error(ctx);
    return folded;
}

int tpdf_save_png(tpdf *t, const char *path, int width, int height, unsigned char *rgb)
{
    fz_context *ctx = t->ctx;
    fz_pixmap *pixmap = NULL;
    int ok = 1;
    fz_var(pixmap);
    fz_var(ok);
    fz_try(ctx) {
        pixmap = fz_new_pixmap_with_data(ctx, fz_device_rgb(ctx), width, height, NULL, 0, width * 3, rgb);
        fz_save_pixmap_as_png(ctx, pixmap, path);
    }
    fz_always(ctx)
        fz_drop_pixmap(ctx, pixmap);
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
