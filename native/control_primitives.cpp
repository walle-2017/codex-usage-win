#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d2d1.h>
#include <d2d1helper.h>
#include <wrl/client.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <mutex>

using Microsoft::WRL::ComPtr;

namespace {

std::once_flag g_control_factory_once;
ComPtr<ID2D1Factory> g_control_factory;

void initialize_control_factory() noexcept
{
    D2D1_FACTORY_OPTIONS options{};
    (void)D2D1CreateFactory(
        D2D1_FACTORY_TYPE_MULTI_THREADED,
        __uuidof(ID2D1Factory),
        &options,
        reinterpret_cast<void**>(g_control_factory.GetAddressOf()));
}

bool ensure_control_factory() noexcept
{
    std::call_once(g_control_factory_once, initialize_control_factory);
    return g_control_factory != nullptr;
}

D2D1_COLOR_F color_from_rgba(
    unsigned char r,
    unsigned char g,
    unsigned char b,
    unsigned char a) noexcept
{
    return D2D1::ColorF(
        static_cast<float>(r) / 255.0f,
        static_cast<float>(g) / 255.0f,
        static_cast<float>(b) / 255.0f,
        static_cast<float>(a) / 255.0f);
}

// Frequently reused rounded controls share one Direct2D target and both
// brushes. BindDC changes per rounded rectangle without recreating resources.
struct RoundedRectCache {
    ComPtr<ID2D1DCRenderTarget> target;
    ComPtr<ID2D1SolidColorBrush> fill;
    ComPtr<ID2D1SolidColorBrush> stroke;
    void reset() noexcept {
        stroke.Reset();
        fill.Reset();
        target.Reset();
    }
};
thread_local RoundedRectCache g_rounded_rect_cache;

// AlphaBlend accepts a reusable source bitmap. Previously each icon painted
// with a new CreateDIBSection, CreateCompatibleDC, SelectObject and teardown.
struct SvgBlendCache {
    HDC source = nullptr;
    HBITMAP bitmap = nullptr;
    HGDIOBJ old_bitmap = nullptr;
    void* bits = nullptr;
    int width = 0;
    int height = 0;

    void reset() noexcept {
        if (source) {
            if (old_bitmap) SelectObject(source, old_bitmap);
            old_bitmap = nullptr;
        }
        if (bitmap) DeleteObject(bitmap);
        bitmap = nullptr;
        bits = nullptr;
        if (source) DeleteDC(source);
        source = nullptr;
        width = height = 0;
    }
    ~SvgBlendCache() { reset(); }

    bool ensure(HDC destination, int w, int h) noexcept {
        if (source && bitmap && bits && width == w && height == h) {
            return true;
        }
        reset();
        source = CreateCompatibleDC(destination);
        if (!source) return false;
        BITMAPINFO info{};
        info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        bitmap = CreateDIBSection(destination, &info, DIB_RGB_COLORS,
                                  &bits, nullptr, 0);
        if (!bitmap || !bits) {
            reset();
            return false;
        }
        old_bitmap = SelectObject(source, bitmap);
        if (!old_bitmap || old_bitmap == HGDI_ERROR) {
            old_bitmap = nullptr;
            reset();
            return false;
        }
        width = w;
        height = h;
        return true;
    }
};
thread_local SvgBlendCache g_svg_blend_cache;

} // namespace

extern "C" __declspec(dllexport) int codex_draw_rounded_rect(
    intptr_t hdc_raw,
    int left,
    int top,
    int right,
    int bottom,
    float radius,
    int fill_enabled,
    unsigned char fill_r,
    unsigned char fill_g,
    unsigned char fill_b,
    unsigned char fill_a,
    int stroke_enabled,
    unsigned char stroke_r,
    unsigned char stroke_g,
    unsigned char stroke_b,
    unsigned char stroke_a,
    float stroke_width) noexcept
{
    HDC hdc = reinterpret_cast<HDC>(hdc_raw);
    const int width = right - left;
    const int height = bottom - top;
    if (!hdc || width <= 0 || height <= 0 || (!fill_enabled && !stroke_enabled)) {
        return 0;
    }
    if (!ensure_control_factory()) {
        return 0;
    }

    try {
        const D2D1_RENDER_TARGET_PROPERTIES properties = D2D1::RenderTargetProperties(
            D2D1_RENDER_TARGET_TYPE_DEFAULT,
            D2D1::PixelFormat(
                DXGI_FORMAT_B8G8R8A8_UNORM,
                D2D1_ALPHA_MODE_IGNORE),
            0.0f,
            0.0f,
            D2D1_RENDER_TARGET_USAGE_NONE,
            D2D1_FEATURE_LEVEL_DEFAULT);

        auto& cache = g_rounded_rect_cache;
        if (!cache.target) {
            HRESULT create_hr = g_control_factory->CreateDCRenderTarget(
                &properties, cache.target.GetAddressOf());
            if (FAILED(create_hr)) return 0;
        }
        auto target = cache.target;
        const RECT bind_rect{left, top, right, bottom};
        HRESULT hr = target->BindDC(hdc, &bind_rect);
        if (FAILED(hr)) {
            cache.reset();
            return 0;
        }
        if (fill_enabled && !cache.fill &&
            FAILED(target->CreateSolidColorBrush(
                color_from_rgba(255, 255, 255, 255), cache.fill.GetAddressOf()))) {
            cache.reset();
            return 0;
        }
        if (stroke_enabled && stroke_width > 0.0f && !cache.stroke &&
            FAILED(target->CreateSolidColorBrush(
                color_from_rgba(255, 255, 255, 255), cache.stroke.GetAddressOf()))) {
            cache.reset();
            return 0;
        }
        target->SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        target->BeginDraw();

        const float safe_radius = std::max(0.5f, radius);
        if (fill_enabled) {
            cache.fill->SetColor(color_from_rgba(fill_r, fill_g, fill_b, fill_a));
            const float inset = 0.35f;
            const D2D1_ROUNDED_RECT shape = D2D1::RoundedRect(
                D2D1::RectF(
                    inset,
                    inset,
                    std::max(inset, static_cast<float>(width) - inset),
                    std::max(inset, static_cast<float>(height) - inset)),
                std::max(0.5f, safe_radius - inset),
                std::max(0.5f, safe_radius - inset));
            target->FillRoundedRectangle(shape, cache.fill.Get());
        }

        if (stroke_enabled && stroke_width > 0.0f) {
            cache.stroke->SetColor(color_from_rgba(stroke_r, stroke_g, stroke_b, stroke_a));
            const float inset = std::max(0.5f, stroke_width * 0.5f);
            const D2D1_ROUNDED_RECT shape = D2D1::RoundedRect(
                D2D1::RectF(
                    inset,
                    inset,
                    std::max(inset, static_cast<float>(width) - inset),
                    std::max(inset, static_cast<float>(height) - inset)),
                std::max(0.5f, safe_radius - inset),
                std::max(0.5f, safe_radius - inset));
            target->DrawRoundedRectangle(
                shape,
                cache.stroke.Get(),
                stroke_width);
        }

        hr = target->EndDraw();
        if (FAILED(hr)) {
            cache.reset();
            return 0;
        }
        return 1;
    } catch (...) {
        return 0;
    }
}


// Composite one cached opaque top-card state into the animation backbuffer.
// AlphaFormat=0: source is a GDI compatible bitmap, not per-pixel alpha.
extern "C" __declspec(dllexport) int codex_alpha_blend_cached_card(
    intptr_t dest_raw, intptr_t source_raw,
    int left, int top, int width, int height, unsigned char opacity) noexcept
{
    HDC dest = reinterpret_cast<HDC>(dest_raw);
    HDC source = reinterpret_cast<HDC>(source_raw);
    if (!dest || !source || width <= 0 || height <= 0) return 0;
    const BLENDFUNCTION blend{AC_SRC_OVER, 0, opacity, 0};
    return AlphaBlend(dest, left, top, width, height,
                      source, left, top, width, height, blend) ? 1 : 0;
}

// SVG icons are rasterized by resvg at the final physical pixel size and
// cached by (icon, DPI, RGBA) in Rust. No hand-authored icon geometry remains.
extern "C" __declspec(dllexport) int codex_blend_settings_icon(
    intptr_t hdc_raw, int left, int top, int width, int height,
    const unsigned char* premultiplied_bgra, size_t buffer_len) noexcept
{
    HDC dest = reinterpret_cast<HDC>(hdc_raw);
    if (!dest || !premultiplied_bgra || width <= 0 || height <= 0 ||
        width > 512 || height > 512 ||
        buffer_len < static_cast<size_t>(width) * static_cast<size_t>(height) * 4) {
        return 0;
    }

    auto& cache = g_svg_blend_cache;
    if (!cache.ensure(dest, width, height)) {
        return 0;
    }
    std::memcpy(cache.bits, premultiplied_bgra,
                static_cast<size_t>(width) * static_cast<size_t>(height) * 4);
    const BLENDFUNCTION blend{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
    const BOOL ok = AlphaBlend(dest, left, top, width, height,
                               cache.source, 0, 0, width, height, blend);
    return ok ? 1 : 0;
}
