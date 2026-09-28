#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <d2d1.h>
#include <d2d1helper.h>
#include <wrl/client.h>

#include <algorithm>
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
