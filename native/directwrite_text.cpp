#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <d2d1.h>
#include <d2d1helper.h>
#include <dwrite.h>
#include <wincodec.h>
#include <wrl/client.h>

#include <algorithm>
#include <cstddef>
#include <mutex>
#include <vector>

using Microsoft::WRL::ComPtr;

namespace {

std::once_flag g_factory_once;
bool g_factories_ready = false;
ComPtr<ID2D1Factory> g_d2d_factory;
ComPtr<IDWriteFactory> g_dwrite_factory;
ComPtr<IWICImagingFactory> g_wic_factory;

void initialize_factories() noexcept
{
    const HRESULT apartment = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(apartment) && apartment != RPC_E_CHANGED_MODE) {
        return;
    }

    if (FAILED(D2D1CreateFactory(
            D2D1_FACTORY_TYPE_SINGLE_THREADED,
            g_d2d_factory.GetAddressOf()))) {
        return;
    }

    if (FAILED(DWriteCreateFactory(
            DWRITE_FACTORY_TYPE_SHARED,
            __uuidof(IDWriteFactory),
            reinterpret_cast<IUnknown**>(g_dwrite_factory.GetAddressOf())))) {
        return;
    }

    if (FAILED(CoCreateInstance(
            CLSID_WICImagingFactory,
            nullptr,
            CLSCTX_INPROC_SERVER,
            IID_PPV_ARGS(g_wic_factory.GetAddressOf())))) {
        return;
    }

    g_factories_ready = true;
}

bool ensure_factories() noexcept
{
    std::call_once(g_factory_once, initialize_factories);
    return g_factories_ready;
}

} // namespace

extern "C" __declspec(dllexport) int codex_directwrite_text_mask(
    const wchar_t* text,
    unsigned int text_len,
    const wchar_t* font_family,
    float font_size_px,
    int font_weight,
    int width,
    int height,
    unsigned char* coverage,
    size_t coverage_len) noexcept
{
    if (!text || !font_family || !coverage || text_len == 0 ||
        font_size_px <= 0.0f || width <= 0 || height <= 0) {
        return 0;
    }

    const size_t pixel_count =
        static_cast<size_t>(width) * static_cast<size_t>(height);
    if (coverage_len < pixel_count || !ensure_factories()) {
        return 0;
    }

    std::fill(coverage, coverage + pixel_count, 0);

    try {
        ComPtr<IWICBitmap> bitmap;
        HRESULT hr = g_wic_factory->CreateBitmap(
            static_cast<UINT>(width),
            static_cast<UINT>(height),
            GUID_WICPixelFormat32bppPBGRA,
            WICBitmapCacheOnLoad,
            bitmap.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        const D2D1_RENDER_TARGET_PROPERTIES properties =
            D2D1::RenderTargetProperties(
                D2D1_RENDER_TARGET_TYPE_DEFAULT,
                D2D1::PixelFormat(
                    DXGI_FORMAT_UNKNOWN,
                    D2D1_ALPHA_MODE_PREMULTIPLIED),
                96.0f,
                96.0f,
                D2D1_RENDER_TARGET_USAGE_NONE,
                D2D1_FEATURE_LEVEL_DEFAULT);

        ComPtr<ID2D1RenderTarget> target;
        hr = g_d2d_factory->CreateWicBitmapRenderTarget(
            bitmap.Get(),
            properties,
            target.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        target->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

        ComPtr<IDWriteRenderingParams> rendering_params;
        if (SUCCEEDED(g_dwrite_factory->CreateRenderingParams(
                rendering_params.GetAddressOf()))) {
            target->SetTextRenderingParams(rendering_params.Get());
        }

        ComPtr<IDWriteTextFormat> format;
        hr = g_dwrite_factory->CreateTextFormat(
            font_family,
            nullptr,
            static_cast<DWRITE_FONT_WEIGHT>(
                std::clamp(font_weight, 1, 999)),
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            font_size_px,
            L"en-us",
            format.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        format->SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
        format->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        format->SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);

        ComPtr<ID2D1SolidColorBrush> brush;
        hr = target->CreateSolidColorBrush(
            D2D1::ColorF(1.0f, 1.0f, 1.0f, 1.0f),
            brush.GetAddressOf());
        if (FAILED(hr)) {
            return 0;
        }

        target->BeginDraw();
        target->Clear(D2D1::ColorF(0.0f, 0.0f));
        target->DrawTextW(
            text,
            text_len,
            format.Get(),
            D2D1::RectF(
                0.0f,
                0.0f,
                static_cast<float>(width),
                static_cast<float>(height)),
            brush.Get(),
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
            DWRITE_MEASURING_MODE_NATURAL);
        hr = target->EndDraw();
        if (FAILED(hr)) {
            return 0;
        }

        const UINT stride = static_cast<UINT>(width) * 4u;
        std::vector<unsigned char> pixels(
            static_cast<size_t>(stride) * static_cast<size_t>(height));
        hr = bitmap->CopyPixels(
            nullptr,
            stride,
            static_cast<UINT>(pixels.size()),
            pixels.data());
        if (FAILED(hr)) {
            return 0;
        }

        for (size_t i = 0; i < pixel_count; ++i) {
            coverage[i] = pixels[i * 4 + 3];
        }
        return 1;
    } catch (...) {
        return 0;
    }
}
