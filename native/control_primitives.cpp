#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d2d1.h>
#include <d2d1helper.h>
#include <wrl/client.h>

#include <algorithm>
#include <cmath>
#include <cstdint>
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
        ComPtr<ID2D1DCRenderTarget> target;
        if (FAILED(g_control_factory->CreateDCRenderTarget(&props, target.GetAddressOf()))) {
            return 0;
        }
        const RECT bind{
            origin_x, origin_y, well_right, well_bottom
        };
        if (FAILED(target->BindDC(hdc, &bind))) {
            return 0;
        }

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
        // Inward turn: concave rounded transition, not a visible panel join.
        sink->AddArc(arc(x0, fy - r, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE));
        sink->AddLine(pt(x0, y0 + r));
        sink->AddArc(arc(x0 + r, y0, clockwise));
        sink->EndFigure(D2D1_FIGURE_END_CLOSED);
        if (FAILED(sink->Close())) {
            return 0;
        }

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
            ComPtr<ID2D1SolidColorBrush> shade;
            if (SUCCEEDED(target->CreateSolidColorBrush(
                color_from_rgba(6, 10, 16, 255), shade.GetAddressOf()))) {
                ComPtr<ID2D1Layer> layer;
                if (SUCCEEDED(target->CreateLayer(nullptr, layer.GetAddressOf()))) {
                    target->PushLayer(D2D1::LayerParameters(
                        D2D1::InfiniteRect(), path.Get(),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE), layer.Get());
                    const float opacity = std::clamp(shadow_opacity, 0.0f, 0.16f);
                    shade->SetOpacity(opacity * 0.38f);
                    target->DrawGeometry(path.Get(), shade.Get(), 8.0f);
                    shade->SetOpacity(opacity * 0.48f);
                    target->DrawGeometry(path.Get(), shade.Get(), 3.0f);
                    target->PopLayer();
                }
            }
        }
        target->PopAxisAlignedClip();
        return SUCCEEDED(target->EndDraw()) ? 1 : 0;
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

        ComPtr<ID2D1DCRenderTarget> target;
        HRESULT hr = g_control_factory->CreateDCRenderTarget(
            &properties,
            target.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        const RECT bind_rect{left, top, right, bottom};
        hr = target->BindDC(hdc, &bind_rect);
        if (FAILED(hr)) {
            return 0;
        }

        target->SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        target->BeginDraw();

        const float safe_radius = std::max(0.5f, radius);
        if (fill_enabled) {
            ComPtr<ID2D1SolidColorBrush> fill_brush;
            hr = target->CreateSolidColorBrush(
                color_from_rgba(fill_r, fill_g, fill_b, fill_a),
                fill_brush.GetAddressOf());
            if (FAILED(hr)) {
                return 0;
            }

            const float inset = 0.35f;
            const D2D1_ROUNDED_RECT shape = D2D1::RoundedRect(
                D2D1::RectF(
                    inset,
                    inset,
                    std::max(inset, static_cast<float>(width) - inset),
                    std::max(inset, static_cast<float>(height) - inset)),
                std::max(0.5f, safe_radius - inset),
                std::max(0.5f, safe_radius - inset));
            target->FillRoundedRectangle(shape, fill_brush.Get());
        }

        if (stroke_enabled && stroke_width > 0.0f) {
            ComPtr<ID2D1SolidColorBrush> stroke_brush;
            hr = target->CreateSolidColorBrush(
                color_from_rgba(stroke_r, stroke_g, stroke_b, stroke_a),
                stroke_brush.GetAddressOf());
            if (FAILED(hr)) {
                return 0;
            }

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
                stroke_brush.Get(),
                stroke_width);
        }

        hr = target->EndDraw();
        return SUCCEEDED(hr) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}


namespace {

template <typename BuildPath>
void draw_icon_path(
    ID2D1Factory* factory,
    ID2D1DCRenderTarget* target,
    ID2D1Brush* brush,
    ID2D1StrokeStyle* stroke_style,
    float stroke_width,
    BuildPath&& build_path)
{
    ComPtr<ID2D1PathGeometry> geometry;
    if (FAILED(factory->CreatePathGeometry(geometry.GetAddressOf()))) {
        return;
    }
    ComPtr<ID2D1GeometrySink> sink;
    if (FAILED(geometry->Open(sink.GetAddressOf()))) {
        return;
    }
    build_path(sink.Get());
    if (FAILED(sink->Close())) {
        return;
    }
    target->DrawGeometry(geometry.Get(), brush, stroke_width, stroke_style);
}

} // namespace

extern "C" __declspec(dllexport) int codex_draw_settings_icon(
    intptr_t hdc_raw,
    int left,
    int top,
    int right,
    int bottom,
    int icon_kind,
    unsigned char color_r,
    unsigned char color_g,
    unsigned char color_b,
    unsigned char color_a) noexcept
{
    HDC hdc = reinterpret_cast<HDC>(hdc_raw);
    const int width = right - left;
    const int height = bottom - top;
    if (!hdc || width <= 0 || height <= 0 || icon_kind < 0 || icon_kind > 31) {
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

        ComPtr<ID2D1DCRenderTarget> target;
        HRESULT hr = g_control_factory->CreateDCRenderTarget(
            &properties,
            target.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        const RECT bind_rect{left, top, right, bottom};
        hr = target->BindDC(hdc, &bind_rect);
        if (FAILED(hr)) {
            return 0;
        }

        ComPtr<ID2D1SolidColorBrush> brush;
        hr = target->CreateSolidColorBrush(
            color_from_rgba(color_r, color_g, color_b, color_a),
            brush.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        D2D1_STROKE_STYLE_PROPERTIES stroke_props = D2D1::StrokeStyleProperties(
            D2D1_CAP_STYLE_ROUND,
            D2D1_CAP_STYLE_ROUND,
            D2D1_CAP_STYLE_ROUND,
            D2D1_LINE_JOIN_ROUND,
            4.0f,
            D2D1_DASH_STYLE_SOLID,
            0.0f);
        ComPtr<ID2D1StrokeStyle> stroke_style;
        hr = g_control_factory->CreateStrokeStyle(
            stroke_props,
            nullptr,
            0,
            stroke_style.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        const float logical_size = 24.0f;
        const float scale = (std::min)(
            static_cast<float>(width),
            static_cast<float>(height)) / logical_size;
        const float origin_x =
            (static_cast<float>(width) - logical_size * scale) * 0.5f;
        const float origin_y =
            (static_cast<float>(height) - logical_size * scale) * 0.5f;
        // Small buttons need a strong silhouette. Icons 26..31 previously fell
        // through to GDI because the entry guard stopped at kind 25.
        const bool compact_ui_icon = icon_kind == 8 || icon_kind >= 26;
        const float stroke_width = (std::max)(
            compact_ui_icon ? 1.85f : 1.25f,
            (compact_ui_icon ? 2.65f : 1.9f) * scale);
        const auto p = [&](float x, float y) noexcept {
            return D2D1::Point2F(origin_x + x * scale, origin_y + y * scale);
        };
        const auto rf = [&](float l, float t, float r, float b) noexcept {
            return D2D1::RectF(
                origin_x + l * scale,
                origin_y + t * scale,
                origin_x + r * scale,
                origin_y + b * scale);
        };
        const auto line = [&](float x1, float y1, float x2, float y2) {
            target->DrawLine(
                p(x1, y1),
                p(x2, y2),
                brush.Get(),
                stroke_width,
                stroke_style.Get());
        };
        const auto ellipse = [&](float cx, float cy, float radius) {
            target->DrawEllipse(
                D2D1::Ellipse(p(cx, cy), radius * scale, radius * scale),
                brush.Get(),
                stroke_width,
                stroke_style.Get());
        };
        const auto dot = [&](float cx, float cy, float radius) {
            target->FillEllipse(
                D2D1::Ellipse(p(cx, cy), radius * scale, radius * scale),
                brush.Get());
        };
        const auto rounded_rect = [&](float l, float t, float r, float b, float radius) {
            target->DrawRoundedRectangle(
                D2D1::RoundedRect(
                    rf(l, t, r, b),
                    radius * scale,
                    radius * scale),
                brush.Get(),
                stroke_width,
                stroke_style.Get());
        };

        target->SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        target->BeginDraw();

        switch (icon_kind) {
        case 0: // General - three tuning sliders.
            line(4.0f, 6.0f, 20.0f, 6.0f);
            line(4.0f, 12.0f, 20.0f, 12.0f);
            line(4.0f, 18.0f, 20.0f, 18.0f);
            ellipse(9.0f, 6.0f, 1.7f);
            ellipse(15.0f, 12.0f, 1.7f);
            ellipse(7.0f, 18.0f, 1.7f);
            break;

        case 1: // Preset - painter palette.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(12.0f, 3.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(7.0f, 3.0f), p(3.0f, 6.5f), p(3.0f, 11.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(3.0f, 15.5f), p(6.5f, 19.0f), p(11.0f, 19.0f)));
                    sink->AddLine(p(12.0f, 19.0f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(13.2f, 19.0f), p(14.0f, 18.2f), p(14.0f, 17.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(14.0f, 16.4f), p(13.8f, 15.9f), p(13.4f, 15.5f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(13.0f, 15.1f), p(12.8f, 14.6f), p(12.8f, 14.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(12.8f, 12.9f), p(13.7f, 12.0f), p(14.8f, 12.0f)));
                    sink->AddLine(p(16.0f, 12.0f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(18.8f, 12.0f), p(21.0f, 10.1f), p(21.0f, 7.8f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(21.0f, 5.1f), p(17.0f, 3.0f), p(12.0f, 3.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_CLOSED);
                });
            dot(8.0f, 8.0f, 1.1f);
            dot(12.0f, 6.2f, 1.1f);
            dot(16.0f, 7.8f, 1.1f);
            break;

        case 2: // Panel.
            rounded_rect(3.5f, 4.0f, 20.5f, 20.0f, 2.5f);
            line(4.5f, 9.0f, 19.5f, 9.0f);
            break;

        case 3: // Tooltip.
            rounded_rect(3.5f, 3.5f, 20.5f, 17.0f, 2.8f);
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(8.0f, 17.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddLine(p(8.0f, 21.0f));
                    sink->AddLine(p(12.5f, 17.0f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 4: // Text.
            line(5.0f, 4.5f, 19.0f, 4.5f);
            line(12.0f, 4.5f, 12.0f, 20.0f);
            break;

        case 5: // Progress - capsule track.
            rounded_rect(3.0f, 8.5f, 21.0f, 15.5f, 3.5f);
            line(7.0f, 12.0f, 16.5f, 12.0f);
            break;

        case 6: // Interaction - cursor with click rays.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(6.0f, 5.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddLine(p(18.5f, 14.0f));
                    sink->AddLine(p(13.1f, 14.8f));
                    sink->AddLine(p(10.8f, 20.0f));
                    sink->AddLine(p(6.0f, 5.0f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(4.0f, 1.8f, 5.1f, 4.0f);
            line(8.0f, 1.0f, 8.0f, 3.5f);
            line(1.5f, 6.0f, 4.0f, 6.5f);
            break;

        case 7: // JSON - drawn braces rather than font glyphs.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(9.0f, 3.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(6.8f, 3.0f), p(6.0f, 4.4f), p(6.0f, 6.5f)));
                    sink->AddLine(p(6.0f, 9.0f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(6.0f, 10.5f), p(5.2f, 11.5f), p(3.5f, 12.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.2f, 12.5f), p(6.0f, 13.5f), p(6.0f, 15.0f)));
                    sink->AddLine(p(6.0f, 17.5f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(6.0f, 19.6f), p(6.8f, 21.0f), p(9.0f, 21.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(15.0f, 3.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(17.2f, 3.0f), p(18.0f, 4.4f), p(18.0f, 6.5f)));
                    sink->AddLine(p(18.0f, 9.0f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(18.0f, 10.5f), p(18.8f, 11.5f), p(20.5f, 12.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(18.8f, 12.5f), p(18.0f, 13.5f), p(18.0f, 15.0f)));
                    sink->AddLine(p(18.0f, 17.5f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(18.0f, 19.6f), p(17.2f, 21.0f), p(15.0f, 21.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 8: // System: recognizable monitor with one simplified gear.
            rounded_rect(2.8f, 3.0f, 17.5f, 15.5f, 2.0f);
            line(9.5f, 15.5f, 9.5f, 18.4f);
            line(6.0f, 18.4f, 12.3f, 18.4f);
            // Four chunky teeth, rather than eight tiny hairlines.
            ellipse(18.1f, 16.9f, 3.25f);
            dot(18.1f, 16.9f, 1.3f);
            line(18.1f, 11.9f, 18.1f, 13.2f);
            line(18.1f, 20.6f, 18.1f, 21.8f);
            line(13.2f, 16.9f, 14.4f, 16.9f);
            line(21.6f, 16.9f, 22.7f, 16.9f);
            break;

        case 9: // Dark - crescent moon.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(18.5f, 15.8f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(16.7f, 16.8f), p(14.4f, 16.7f), p(12.5f, 15.5f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(8.6f, 13.1f), p(7.2f, 8.1f), p(9.7f, 4.3f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(10.2f, 3.5f), p(10.9f, 2.8f), p(11.8f, 2.3f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(6.8f, 2.4f), p(2.8f, 6.6f), p(2.8f, 11.7f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(2.8f, 17.0f), p(7.1f, 21.3f), p(12.4f, 21.3f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(16.3f, 21.3f), p(19.7f, 19.0f), p(21.2f, 15.8f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(20.3f, 16.0f), p(19.4f, 16.1f), p(18.5f, 15.8f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 10: // Light - sun.
            ellipse(12.0f, 12.0f, 4.0f);
            line(12.0f, 2.0f, 12.0f, 4.5f);
            line(12.0f, 19.5f, 12.0f, 22.0f);
            line(2.0f, 12.0f, 4.5f, 12.0f);
            line(19.5f, 12.0f, 22.0f, 12.0f);
            line(4.9f, 4.9f, 6.7f, 6.7f);
            line(17.3f, 17.3f, 19.1f, 19.1f);
            line(4.9f, 19.1f, 6.7f, 17.3f);
            line(17.3f, 6.7f, 19.1f, 4.9f);
            break;

        case 11: // Default layout - full two-row usage panel.
            rounded_rect(3.0f, 3.0f, 21.0f, 21.0f, 2.8f);
            line(5.5f, 8.0f, 9.0f, 8.0f);
            line(11.0f, 8.0f, 18.5f, 8.0f);
            line(5.5f, 16.0f, 9.0f, 16.0f);
            line(11.0f, 16.0f, 18.5f, 16.0f);
            line(5.5f, 11.5f, 18.5f, 11.5f);
            break;

        case 12: // Minimal layout - one compact usage row.
            rounded_rect(3.0f, 7.0f, 21.0f, 17.0f, 2.8f);
            line(5.5f, 12.0f, 9.0f, 12.0f);
            line(11.0f, 12.0f, 18.5f, 12.0f);
            break;

        case 13: // Dark Classic - isometric graphite cube.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(12.0f, 2.5f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddLine(p(20.0f, 7.0f));
                    sink->AddLine(p(20.0f, 16.5f));
                    sink->AddLine(p(12.0f, 21.5f));
                    sink->AddLine(p(4.0f, 16.5f));
                    sink->AddLine(p(4.0f, 7.0f));
                    sink->AddLine(p(12.0f, 2.5f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(4.2f, 7.0f, 12.0f, 11.7f);
            line(19.8f, 7.0f, 12.0f, 11.7f);
            line(12.0f, 11.7f, 12.0f, 21.0f);
            break;

        case 14: // Dark Ocean - three smooth waves.
            for (float y : {6.0f, 12.0f, 18.0f}) {
                draw_icon_path(
                    g_control_factory.Get(),
                    target.Get(),
                    brush.Get(),
                    stroke_style.Get(),
                    stroke_width,
                    [&](ID2D1GeometrySink* sink) {
                        sink->BeginFigure(p(3.0f, y), D2D1_FIGURE_BEGIN_HOLLOW);
                        sink->AddBezier(D2D1::BezierSegment(
                            p(5.0f, y - 2.0f), p(7.0f, y - 2.0f), p(9.0f, y)));
                        sink->AddBezier(D2D1::BezierSegment(
                            p(11.0f, y + 2.0f), p(13.0f, y + 2.0f), p(15.0f, y)));
                        sink->AddBezier(D2D1::BezierSegment(
                            p(17.0f, y - 2.0f), p(19.0f, y - 2.0f), p(21.0f, y)));
                        sink->EndFigure(D2D1_FIGURE_END_OPEN);
                    });
            }
            break;

        case 15: // Dark Forest - pine tree.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(12.0f, 2.5f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddLine(p(6.0f, 10.0f));
                    sink->AddLine(p(9.0f, 10.0f));
                    sink->AddLine(p(4.5f, 16.0f));
                    sink->AddLine(p(10.0f, 16.0f));
                    sink->AddLine(p(10.0f, 21.0f));
                    sink->AddLine(p(14.0f, 21.0f));
                    sink->AddLine(p(14.0f, 16.0f));
                    sink->AddLine(p(19.5f, 16.0f));
                    sink->AddLine(p(15.0f, 10.0f));
                    sink->AddLine(p(18.0f, 10.0f));
                    sink->AddLine(p(12.0f, 2.5f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 16: // Custom preset - diagonal paintbrush.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(14.3f, 3.5f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddLine(p(20.2f, 9.4f));
                    sink->AddLine(p(12.2f, 17.4f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(10.8f, 18.8f), p(8.6f, 18.8f), p(7.2f, 17.4f)));
                    sink->AddLine(p(6.6f, 16.8f));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.2f, 15.4f), p(5.2f, 13.2f), p(6.6f, 11.8f)));
                    sink->AddLine(p(14.3f, 3.5f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(11.7f, 6.2f, 17.6f, 12.1f);
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(7.4f, 17.6f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(6.5f, 20.0f), p(4.3f, 21.0f), p(2.6f, 20.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(4.1f, 19.4f), p(4.6f, 18.0f), p(4.8f, 16.8f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 17: // Light Cloud Porcelain - cloud.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(5.0f, 17.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(2.8f, 17.0f), p(2.5f, 13.4f), p(4.7f, 12.4f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.3f, 8.8f), p(8.5f, 6.4f), p(12.0f, 7.6f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(13.8f, 5.6f), p(17.6f, 6.4f), p(18.2f, 9.6f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(21.4f, 10.0f), p(22.0f, 15.8f), p(18.0f, 17.0f)));
                    sink->AddLine(p(5.0f, 17.0f));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 18: // Light Clear Bay - sun over calm bay.
            line(3.0f, 15.0f, 21.0f, 15.0f);
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(4.0f, 19.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(7.0f, 16.5f), p(9.0f, 21.5f), p(12.0f, 19.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(15.0f, 16.5f), p(17.0f, 21.5f), p(20.0f, 19.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(7.0f, 15.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(7.3f, 10.7f), p(9.2f, 8.3f), p(12.0f, 8.3f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(14.8f, 8.3f), p(16.7f, 10.7f), p(17.0f, 15.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(12.0f, 4.0f, 12.0f, 6.0f);
            line(5.3f, 7.3f, 7.0f, 8.7f);
            line(18.7f, 7.3f, 17.0f, 8.7f);
            break;

        case 19: // Light Wheat Glow - wheat ear.
            line(12.0f, 21.0f, 12.0f, 5.0f);
            line(12.0f, 18.0f, 8.0f, 15.0f);
            line(12.0f, 15.0f, 16.0f, 12.0f);
            line(12.0f, 12.0f, 8.5f, 9.0f);
            line(12.0f, 9.0f, 15.5f, 6.0f);
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(8.0f, 15.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.7f, 14.7f), p(4.8f, 13.0f), p(5.2f, 11.5f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(7.0f, 11.7f), p(8.0f, 12.8f), p(8.0f, 15.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(16.0f, 12.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(18.3f, 11.7f), p(19.2f, 10.0f), p(18.8f, 8.5f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(17.0f, 8.7f), p(16.0f, 9.8f), p(16.0f, 12.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            break;

        case 20: // JSON Reload - circular arrow.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(18.5f, 8.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(16.6f, 4.4f), p(12.0f, 3.0f), p(8.0f, 5.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(4.0f, 7.0f), p(3.0f, 12.5f), p(5.7f, 16.2f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(8.3f, 19.9f), p(13.5f, 20.6f), p(17.0f, 17.5f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(18.5f, 8.0f, 18.5f, 3.8f);
            line(18.5f, 8.0f, 14.3f, 8.0f);
            break;

        case 21: // JSON Format - braces with formatted lines.
            draw_icon_path(
                g_control_factory.Get(),
                target.Get(),
                brush.Get(),
                stroke_style.Get(),
                stroke_width,
                [&](ID2D1GeometrySink* sink) {
                    sink->BeginFigure(p(7.5f, 4.0f), D2D1_FIGURE_BEGIN_HOLLOW);
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.8f, 4.0f), p(5.5f, 5.4f), p(5.5f, 7.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.5f, 9.5f), p(4.5f, 11.0f), p(3.0f, 12.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(4.5f, 13.0f), p(5.5f, 14.5f), p(5.5f, 17.0f)));
                    sink->AddBezier(D2D1::BezierSegment(
                        p(5.5f, 18.6f), p(5.8f, 20.0f), p(7.5f, 20.0f)));
                    sink->EndFigure(D2D1_FIGURE_END_OPEN);
                });
            line(10.0f, 7.0f, 20.0f, 7.0f);
            line(10.0f, 12.0f, 17.0f, 12.0f);
            line(10.0f, 17.0f, 19.0f, 17.0f);
            break;

        case 22: // JSON Import - arrow into tray.
            line(12.0f, 3.0f, 12.0f, 14.0f);
            line(8.0f, 10.0f, 12.0f, 14.0f);
            line(16.0f, 10.0f, 12.0f, 14.0f);
            line(5.0f, 17.0f, 5.0f, 20.0f);
            line(5.0f, 20.0f, 19.0f, 20.0f);
            line(19.0f, 20.0f, 19.0f, 17.0f);
            break;

        case 23: // JSON Export - arrow out of tray.
            line(12.0f, 14.0f, 12.0f, 3.0f);
            line(8.0f, 7.0f, 12.0f, 3.0f);
            line(16.0f, 7.0f, 12.0f, 3.0f);
            line(5.0f, 17.0f, 5.0f, 20.0f);
            line(5.0f, 20.0f, 19.0f, 20.0f);
            line(19.0f, 20.0f, 19.0f, 17.0f);
            break;

        case 24: // JSON Apply - check in circle.
            ellipse(12.0f, 12.0f, 8.5f);
            line(7.5f, 12.2f, 10.5f, 15.2f);
            line(10.5f, 15.2f, 16.8f, 8.7f);
            break;
        case 25: // Adaptive layout - progressively compact rows.
            line(4.0f, 6.0f, 20.0f, 6.0f);
            line(6.0f, 12.0f, 18.0f, 12.0f);
            line(8.0f, 18.0f, 16.0f, 18.0f);
            break;

        case 26: // Horizontal: one strong wide silhouette.
            rounded_rect(3.6f, 6.2f, 20.4f, 17.8f, 2.8f);
            line(1.6f, 8.5f, 1.6f, 15.5f);
            line(22.4f, 8.5f, 22.4f, 15.5f);
            break;

        case 27: // Vertical: one strong tall silhouette.
            rounded_rect(7.3f, 3.6f, 16.7f, 20.4f, 2.8f);
            line(8.5f, 1.6f, 15.5f, 1.6f);
            line(8.5f, 22.4f, 15.5f, 22.4f);
            break;

        case 28: // On: soft capsule, large solid thumb on the RIGHT.
        case 29: { // Off: same capsule, thumb on the LEFT.
            const auto outer = D2D1::RoundedRect(
                rf(2.0f, 6.1f, 22.0f, 17.9f), 5.9f*scale, 5.9f*scale);
            brush->SetOpacity(0.34f);
            target->FillRoundedRectangle(outer, brush.Get());
            brush->SetOpacity(0.80f);
            target->DrawRoundedRectangle(outer, brush.Get(), stroke_width*0.78f);
            brush->SetOpacity(1.0f);
            dot(icon_kind == 28 ? 16.2f : 7.8f, 12.0f, 4.1f);
            break;
        }

        case 30: // Taskbar: a single wide status strip with TWO clear blocks.
            rounded_rect(2.2f, 7.0f, 21.8f, 17.0f, 2.2f);
            line(5.0f, 12.0f, 9.0f, 12.0f);
            dot(15.8f, 12.0f, 1.75f);
            dot(19.5f, 12.0f, 1.75f);
            break;

        case 31: // Desktop: four large app tiles (NO monitor or gear).
            // The 2x2 grid differentiates Desktop from Theme/System.
            for (const float tx : {3.0f, 13.0f}) {
                for (const float ty : {3.0f, 13.0f}) {
                    target->FillRoundedRectangle(
                        D2D1::RoundedRect(rf(tx,ty,tx+8.0f,ty+8.0f),
                                          1.7f*scale,1.7f*scale),
                        brush.Get());
                }
            }
            break;

        default:
            break;
        }

        hr = target->EndDraw();
        return SUCCEEDED(hr) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}
