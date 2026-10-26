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

// The settings submenu renders repeatedly during its 200-ms animation.
// Cache device-dependent drawing objects on the UI thread; rebuild geometry
// only when DPI or menu bounds change, and discard everything on device loss.
struct DesktopInsetCache {
    ComPtr<ID2D1DCRenderTarget> target;
    ComPtr<ID2D1PathGeometry> path;
    ComPtr<ID2D1Layer> layer;
    ComPtr<ID2D1SolidColorBrush> shadow;
    float x0 = -1.0f, x1 = -1.0f, y1 = -1.0f, fy = -1.0f, radius = -1.0f;

    void reset() noexcept {
        shadow.Reset();
        layer.Reset();
        path.Reset();
        target.Reset();
        x0 = x1 = y1 = fy = radius = -1.0f;
    }
};

thread_local DesktopInsetCache g_desktop_inset_cache;

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

// One continuous, antialiased L-shaped inset used by the settings page.
// The recessed foot surrounds the Desktop selection and joins the right well
// at a *concave* rounded corner. No overlapping rectangular shadow strips.
extern "C" __declspec(dllexport) int codex_draw_desktop_inset_well(
    intptr_t hdc_raw,
    int well_left, int well_top, int well_right, int well_bottom,
    int foot_left, int foot_top, int foot_right, int foot_bottom,
    int reveal_right, float radius,
    unsigned char top_r, unsigned char top_g, unsigned char top_b,
    unsigned char bottom_r, unsigned char bottom_g, unsigned char bottom_b,
    float shadow_opacity) noexcept
{
    HDC hdc = reinterpret_cast<HDC>(hdc_raw);
    if (!hdc || !ensure_control_factory()
        || foot_left >= well_left || foot_top <= well_top
        || foot_right <= well_left || foot_right >= well_right
        || foot_bottom != well_bottom || well_right <= well_left
        || well_bottom <= foot_top || reveal_right <= foot_left) {
        return 0;
    }

    try {
        const int origin_x = foot_left;
        const int origin_y = well_top;
        const float x0 = static_cast<float>(well_left - origin_x);
        const float y0 = 0.0f;
        const float x1 = static_cast<float>(well_right - origin_x);
        const float y1 = static_cast<float>(well_bottom - origin_y);
        const float fx = 0.0f;
        const float fy = static_cast<float>(foot_top - origin_y);
        const float r = std::clamp(radius, 2.0f, 16.0f);

        const D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties(
            D2D1_RENDER_TARGET_TYPE_DEFAULT,
            D2D1::PixelFormat(DXGI_FORMAT_B8G8R8A8_UNORM, D2D1_ALPHA_MODE_IGNORE),
            0.0f, 0.0f, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_FEATURE_LEVEL_DEFAULT);
        auto& cache = g_desktop_inset_cache;
        if (!cache.target && FAILED(g_control_factory->CreateDCRenderTarget(
            &props, cache.target.GetAddressOf()))) {
            return 0;
        }
        ComPtr<ID2D1DCRenderTarget> target = cache.target;
        const RECT bind{ origin_x, origin_y, well_right, well_bottom };
        if (FAILED(target->BindDC(hdc, &bind))) {
            cache.reset();
            return 0;
        }

        if (!cache.path || cache.x0 != x0 || cache.x1 != x1 ||
            cache.y1 != y1 || cache.fy != fy || cache.radius != r) {
            ComPtr<ID2D1PathGeometry> path;
            ComPtr<ID2D1GeometrySink> sink;
            if (FAILED(g_control_factory->CreatePathGeometry(path.GetAddressOf()))
                || FAILED(path->Open(sink.GetAddressOf()))) {
                return 0;
            }
            const auto pt = [](float x, float y) { return D2D1::Point2F(x, y); };
            const auto arc = [r](float x, float y, D2D1_SWEEP_DIRECTION dir) {
                return D2D1::ArcSegment(
                    D2D1::Point2F(x, y), D2D1::SizeF(r, r), 0.0f, dir,
                    D2D1_ARC_SIZE_SMALL);
            };
            const auto clockwise = D2D1_SWEEP_DIRECTION_CLOCKWISE;
            sink->BeginFigure(pt(x0 + r, y0), D2D1_FIGURE_BEGIN_FILLED);
            sink->AddLine(pt(x1 - r, y0));
            sink->AddArc(arc(x1, y0 + r, clockwise));
            sink->AddLine(pt(x1, y1 - r));
            sink->AddArc(arc(x1 - r, y1, clockwise));
            sink->AddLine(pt(fx + r, y1));
            sink->AddArc(arc(fx, y1 - r, clockwise));
            sink->AddLine(pt(fx, fy + r));
            sink->AddArc(arc(fx + r, fy, clockwise));
            sink->AddLine(pt(x0 - r, fy));
            sink->AddArc(arc(x0, fy - r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
            sink->AddLine(pt(x0, y0 + r));
            sink->AddArc(arc(x0 + r, y0, clockwise));
            sink->EndFigure(D2D1_FIGURE_END_CLOSED);
            if (FAILED(sink->Close())) {
                return 0;
            }
            cache.path = path;
            cache.x0 = x0; cache.x1 = x1;
            cache.y1 = y1; cache.fy = fy; cache.radius = r;
        }
        ComPtr<ID2D1PathGeometry> path = cache.path;
        const auto pt = [](float x, float y) { return D2D1::Point2F(x, y); };

        const D2D1_GRADIENT_STOP stops[] = {
            { 0.0f, color_from_rgba(top_r, top_g, top_b, 255) },
            { 1.0f, color_from_rgba(bottom_r, bottom_g, bottom_b, 255) }
        };
        ComPtr<ID2D1GradientStopCollection> gradient_stops;
        if (FAILED(target->CreateGradientStopCollection(
            stops, 2, gradient_stops.GetAddressOf()))) {
            return 0;
        }
        const D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES gradient_props{
            pt(x0, y0), pt(x0, y1)
        };
        ComPtr<ID2D1LinearGradientBrush> brush;
        if (FAILED(target->CreateLinearGradientBrush(
            &gradient_props, nullptr, gradient_stops.Get(),
            brush.GetAddressOf()))) {
            return 0;
        }

        target->BeginDraw();
        const float reveal_x = std::clamp(
            static_cast<float>(reveal_right - origin_x), 0.0f, x1);
        target->PushAxisAlignedClip(
            D2D1::RectF(0.0f, 0.0f, reveal_x, y1),
            D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        target->FillGeometry(path.Get(), brush.Get());

        // Very low-opacity *interior* shadow only. Geometry masking clips
        // strokes at the exact same outline so no dark outer border appears.
        if (shadow_opacity > 0.001f) {
            if (!cache.shadow) {
                (void)target->CreateSolidColorBrush(
                    color_from_rgba(6, 10, 16, 255), cache.shadow.GetAddressOf());
            }
            if (!cache.layer) {
                (void)target->CreateLayer(nullptr, cache.layer.GetAddressOf());
            }
            if (cache.shadow && cache.layer) {
                target->PushLayer(D2D1::LayerParameters(
                    D2D1::InfiniteRect(), path.Get(),
                    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE), cache.layer.Get());
                const float opacity = std::clamp(shadow_opacity, 0.0f, 0.16f);
                cache.shadow->SetOpacity(opacity * 0.38f);
                target->DrawGeometry(path.Get(), cache.shadow.Get(), 8.0f);
                cache.shadow->SetOpacity(opacity * 0.48f);
                target->DrawGeometry(path.Get(), cache.shadow.Get(), 3.0f);
                target->PopLayer();
            }
        }
        target->PopAxisAlignedClip();
        const HRESULT result = target->EndDraw();
        if (FAILED(result)) {
            cache.reset();
            return 0;
        }
        return 1;
    } catch (...) {
        return 0;
    }
}

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
