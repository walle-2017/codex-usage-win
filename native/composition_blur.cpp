#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <roapi.h>
#include <DispatcherQueue.h>
#include <initguid.h>
#include <d2d1effects.h>
#include <windows.graphics.effects.interop.h>
#include <windows.ui.composition.interop.h>

#include <algorithm>
#include <string_view>

#include <winrt/base.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.Foundation.Numerics.h>
#include <winrt/Windows.Graphics.Effects.h>
#include <winrt/Windows.System.h>
#include <winrt/Windows.UI.h>
#include <winrt/Windows.UI.Composition.h>
#include <winrt/Windows.UI.Composition.Desktop.h>

namespace wf = winrt::Windows::Foundation;
namespace wge = winrt::Windows::Graphics::Effects;
namespace wuc = winrt::Windows::UI::Composition;
namespace wucd = winrt::Windows::UI::Composition::Desktop;
namespace awge = ABI::Windows::Graphics::Effects;
namespace awucd = ABI::Windows::UI::Composition::Desktop;

namespace {

thread_local winrt::Windows::System::DispatcherQueueController g_dispatcher_controller{nullptr};
thread_local bool g_ro_initialized = false;

bool ensure_winrt_and_dispatcher() noexcept
{
    // Initialize the apartment at most once for this UI thread and tolerate an
    // apartment that another component already initialized with a different model.
    if (!g_ro_initialized) {
        const HRESULT ro = RoInitialize(RO_INIT_SINGLETHREADED);
        if (FAILED(ro) && ro != RPC_E_CHANGED_MODE) {
            return false;
        }
        g_ro_initialized = true;
    }

    try {
        if (winrt::Windows::System::DispatcherQueue::GetForCurrentThread()) {
            return true;
        }
    } catch (...) {
        // Fall through and create one explicitly.
    }

    if (g_dispatcher_controller) {
        return true;
    }

    DispatcherQueueOptions options{
        sizeof(DispatcherQueueOptions),
        DQTYPE_THREAD_CURRENT,
        DQTAT_COM_ASTA,
    };

    winrt::Windows::System::DispatcherQueueController controller{nullptr};
    const HRESULT hr = CreateDispatcherQueueController(
        options,
        reinterpret_cast<ABI::Windows::System::IDispatcherQueueController**>(
            winrt::put_abi(controller)));
    if (FAILED(hr)) {
        return false;
    }

    g_dispatcher_controller = controller;
    return true;
}

struct GaussianBlurEffect :
    winrt::implements<
        GaussianBlurEffect,
        wge::IGraphicsEffect,
        wge::IGraphicsEffectSource,
        awge::IGraphicsEffectD2D1Interop>
{
    winrt::hstring Name() const
    {
        return m_name;
    }

    void Name(winrt::hstring const& value)
    {
        m_name = value;
    }

    HRESULT STDMETHODCALLTYPE GetEffectId(GUID* id) noexcept override
    {
        if (!id) {
            return E_INVALIDARG;
        }
        *id = CLSID_D2D1GaussianBlur;
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE GetNamedPropertyMapping(
        LPCWSTR name,
        UINT* index,
        awge::GRAPHICS_EFFECT_PROPERTY_MAPPING* mapping) noexcept override
    {
        if (!name || !index || !mapping) {
            return E_INVALIDARG;
        }

        const std::wstring_view property{name};
        if (property == L"BlurAmount") {
            *index = D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION;
            *mapping = awge::GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            return S_OK;
        }
        if (property == L"Optimization") {
            *index = D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION;
            *mapping = awge::GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            return S_OK;
        }
        if (property == L"BorderMode") {
            *index = D2D1_GAUSSIANBLUR_PROP_BORDER_MODE;
            *mapping = awge::GRAPHICS_EFFECT_PROPERTY_MAPPING_DIRECT;
            return S_OK;
        }
        return E_INVALIDARG;
    }

    HRESULT STDMETHODCALLTYPE GetPropertyCount(UINT* count) noexcept override
    {
        if (!count) {
            return E_INVALIDARG;
        }
        *count = 3;
        return S_OK;
    }

    HRESULT STDMETHODCALLTYPE GetProperty(
        UINT index,
        ABI::Windows::Foundation::IPropertyValue** value) noexcept override
    {
        if (!value) {
            return E_INVALIDARG;
        }
        *value = nullptr;

        try {
            switch (index) {
            case D2D1_GAUSSIANBLUR_PROP_STANDARD_DEVIATION:
                *value = wf::PropertyValue::CreateSingle(blur_amount)
                             .as<ABI::Windows::Foundation::IPropertyValue>()
                             .detach();
                break;
            case D2D1_GAUSSIANBLUR_PROP_OPTIMIZATION:
                *value = wf::PropertyValue::CreateUInt32(
                             D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED)
                             .as<ABI::Windows::Foundation::IPropertyValue>()
                             .detach();
                break;
            case D2D1_GAUSSIANBLUR_PROP_BORDER_MODE:
                *value = wf::PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD)
                             .as<ABI::Windows::Foundation::IPropertyValue>()
                             .detach();
                break;
            default:
                return E_BOUNDS;
            }
            return S_OK;
        } catch (...) {
            return winrt::to_hresult();
        }
    }

    HRESULT STDMETHODCALLTYPE GetSource(
        UINT index,
        awge::IGraphicsEffectSource** source) noexcept override
    {
        if (!source) {
            return E_INVALIDARG;
        }
        *source = nullptr;
        if (index != 0 || !Source) {
            return E_BOUNDS;
        }

        try {
            winrt::copy_to_abi(Source, *reinterpret_cast<void**>(source));
            return S_OK;
        } catch (...) {
            return winrt::to_hresult();
        }
    }

    HRESULT STDMETHODCALLTYPE GetSourceCount(UINT* count) noexcept override
    {
        if (!count) {
            return E_INVALIDARG;
        }
        *count = 1;
        return S_OK;
    }

    wge::IGraphicsEffectSource Source{nullptr};
    float blur_amount = 0.0f;

private:
    winrt::hstring m_name{L"Blur"};
};

struct CompositionBlurContext
{
    wuc::Compositor compositor{nullptr};
    wucd::DesktopWindowTarget target{nullptr};
    wuc::ContainerVisual root{nullptr};
    wuc::ContainerVisual panel_visual{nullptr};
    wuc::ContainerVisual grip_visual{nullptr};
    wuc::CompositionRoundedRectangleGeometry panel_geometry{nullptr};
    wuc::CompositionRoundedRectangleGeometry grip_geometry{nullptr};
    wuc::SpriteVisual blur_visual{nullptr};
    wuc::SpriteVisual tint_visual{nullptr};
    wuc::SpriteVisual grip_blur_visual{nullptr};
    wuc::SpriteVisual grip_tint_visual{nullptr};
    wuc::CompositionEffectBrush blur_brush{nullptr};
    wuc::CompositionColorBrush tint_brush{nullptr};

    explicit CompositionBlurContext(HWND hwnd, float blur_amount, BYTE r, BYTE g, BYTE b, BYTE a)
    {
        compositor = wuc::Compositor();

        auto interop = compositor.as<awucd::ICompositorDesktopInterop>();
        winrt::check_hresult(interop->CreateDesktopWindowTarget(
            hwnd,
            FALSE,
            reinterpret_cast<awucd::IDesktopWindowTarget**>(winrt::put_abi(target))));

        auto source_parameter = wuc::CompositionEffectSourceParameter(L"backdrop");
        auto effect = winrt::make_self<GaussianBlurEffect>();
        effect->blur_amount = std::clamp(blur_amount, 0.0f, 250.0f);
        effect->Source = source_parameter;

        auto animatable_properties =
            winrt::single_threaded_vector<winrt::hstring>();
        animatable_properties.Append(L"Blur.BlurAmount");
        auto factory = compositor.CreateEffectFactory(
            effect.as<wge::IGraphicsEffect>(),
            animatable_properties);
        blur_brush = factory.CreateBrush();

        // For a WS_EX_NOREDIRECTIONBITMAP Win32 target, CreateBackdropBrush
        // samples the real composition content behind this transparent window.
        auto backdrop = compositor.CreateBackdropBrush();
        blur_brush.SetSourceParameter(L"backdrop", backdrop);

        root = compositor.CreateContainerVisual();
        panel_visual = compositor.CreateContainerVisual();
        grip_visual = compositor.CreateContainerVisual();
        panel_geometry = compositor.CreateRoundedRectangleGeometry();
        grip_geometry = compositor.CreateRoundedRectangleGeometry();
        panel_visual.Clip(compositor.CreateGeometricClip(panel_geometry));
        grip_visual.Clip(compositor.CreateGeometricClip(grip_geometry));

        tint_brush = compositor.CreateColorBrush(winrt::Windows::UI::Color{a, r, g, b});
        blur_visual = compositor.CreateSpriteVisual();
        blur_visual.Brush(blur_brush);
        panel_visual.Children().InsertAtBottom(blur_visual);
        tint_visual = compositor.CreateSpriteVisual();
        tint_visual.Brush(tint_brush);
        panel_visual.Children().InsertAtTop(tint_visual);

        // Same Gaussian backdrop and tint brushes, but a separate rounded
        // clip and opacity for the external desktop drag grip.
        grip_blur_visual = compositor.CreateSpriteVisual();
        grip_blur_visual.Brush(blur_brush);
        grip_visual.Children().InsertAtBottom(grip_blur_visual);
        grip_tint_visual = compositor.CreateSpriteVisual();
        grip_tint_visual.Brush(tint_brush);
        grip_visual.Children().InsertAtTop(grip_tint_visual);
        root.Children().InsertAtBottom(panel_visual);
        root.Children().InsertAtTop(grip_visual);
        grip_visual.IsVisible(false);

        // Do not rely on DesktopWindowTarget's asynchronous resize propagation.
        // Keep the entire visual tree explicitly bounded and clipped.
        set_bounds(1.0f, 1.0f, 0.0f, 0.0f);
        target.Root(root);
    }

    static void set_clip(
        wuc::CompositionRoundedRectangleGeometry const& geometry,
        float width, float height, float clip_inset, float clip_radius)
    {
        const float safe_width = (std::max)(1.0f, width);
        const float safe_height = (std::max)(1.0f, height);
        // The layered foreground paints the border; only the inner fill
        // receives the composition backdrop.
        const float max_inset = (std::min)(safe_width, safe_height) * 0.5f;
        const float inset = std::clamp(clip_inset, 0.0f, max_inset);
        const float clip_width = (std::max)(1.0f, safe_width - inset * 2.0f);
        const float clip_height = (std::max)(1.0f, safe_height - inset * 2.0f);
        const float radius = std::clamp(
            clip_radius, 0.0f, (std::min)(clip_width, clip_height) * 0.5f);
        geometry.Offset({inset, inset});
        geometry.Size({clip_width, clip_height});
        geometry.CornerRadius({radius, radius});
    }

    void set_bounds(float width, float height, float clip_inset, float clip_radius)
    {
        const winrt::Windows::Foundation::Numerics::float2 size{
            (std::max)(1.0f, width), (std::max)(1.0f, height)};
        root.Size(size);
        panel_visual.Offset({0.0f, 0.0f, 0.0f});
        panel_visual.Size(size);
        blur_visual.Size(size);
        tint_visual.Size(size);
        set_clip(panel_geometry, size.x, size.y, clip_inset, clip_radius);
        panel_visual.IsVisible(true);
        grip_visual.IsVisible(false);
    }

    void set_desktop_bounds(
        float canvas_w, float canvas_h,
        float panel_x, float panel_y, float panel_w, float panel_h,
        float grip_x, float grip_y, float grip_w, float grip_h,
        float panel_inset, float panel_radius,
        float grip_inset, float grip_radius,
        float grip_opacity, bool panel_visible)
    {
        root.Size({(std::max)(1.0f, canvas_w), (std::max)(1.0f, canvas_h)});
        const winrt::Windows::Foundation::Numerics::float2 panel_size{
            (std::max)(1.0f, panel_w), (std::max)(1.0f, panel_h)};
        panel_visual.Offset({panel_x, panel_y, 0.0f});
        panel_visual.Size(panel_size);
        blur_visual.Size(panel_size);
        tint_visual.Size(panel_size);
        set_clip(panel_geometry, panel_size.x, panel_size.y, panel_inset, panel_radius);
        panel_visual.IsVisible(panel_visible);

        const winrt::Windows::Foundation::Numerics::float2 grip_size{
            (std::max)(1.0f, grip_w), (std::max)(1.0f, grip_h)};
        grip_visual.Offset({grip_x, grip_y, 0.0f});
        grip_visual.Size(grip_size);
        grip_blur_visual.Size(grip_size);
        grip_tint_visual.Size(grip_size);
        set_clip(grip_geometry, grip_size.x, grip_size.y, grip_inset, grip_radius);
        const float alpha = std::clamp(grip_opacity, 0.0f, 1.0f);
        grip_visual.Opacity(alpha);
        grip_visual.IsVisible(alpha > 0.0f);
    }

    void set_blur(float amount)
    {
        blur_brush.Properties().InsertScalar(
            L"Blur.BlurAmount",
            std::clamp(amount, 0.0f, 250.0f));
    }

    void set_tint(BYTE r, BYTE g, BYTE b, BYTE a)
    {
        tint_brush.Color(winrt::Windows::UI::Color{a, r, g, b});
    }
};

} // namespace

extern "C" __declspec(dllexport) void* codex_composition_blur_create(
    intptr_t hwnd_raw,
    float blur_amount,
    unsigned char r,
    unsigned char g,
    unsigned char b,
    unsigned char a) noexcept
{
    if (!hwnd_raw || !ensure_winrt_and_dispatcher()) {
        return nullptr;
    }

    try {
        return new CompositionBlurContext(
            reinterpret_cast<HWND>(hwnd_raw),
            blur_amount,
            r,
            g,
            b,
            a);
    } catch (...) {
        return nullptr;
    }
}

extern "C" __declspec(dllexport) int codex_composition_blur_set_bounds(
    void* context,
    float width,
    float height,
    float clip_inset,
    float clip_radius) noexcept
{
    if (!context || width <= 0.0f || height <= 0.0f) {
        return 0;
    }
    try {
        static_cast<CompositionBlurContext*>(context)->set_bounds(
            width,
            height,
            clip_inset,
            clip_radius);
        return 1;
    } catch (...) {
        return 0;
    }
}

extern "C" __declspec(dllexport) int codex_composition_blur_set_desktop_bounds(
    void* context, float canvas_w, float canvas_h,
    float panel_x, float panel_y, float panel_w, float panel_h,
    float grip_x, float grip_y, float grip_w, float grip_h,
    float panel_inset, float panel_radius, float grip_inset, float grip_radius,
    float grip_opacity, int panel_visible) noexcept
{
    if (!context || canvas_w <= 0.0f || canvas_h <= 0.0f) {
        return 0;
    }
    try {
        static_cast<CompositionBlurContext*>(context)->set_desktop_bounds(
            canvas_w, canvas_h, panel_x, panel_y, panel_w, panel_h,
            grip_x, grip_y, grip_w, grip_h,
            panel_inset, panel_radius, grip_inset, grip_radius,
            grip_opacity, panel_visible != 0);
        return 1;
    } catch (...) {
        return 0;
    }
}

extern "C" __declspec(dllexport) int codex_composition_blur_set_amount(
    void* context,
    float blur_amount) noexcept
{
    if (!context) {
        return 0;
    }
    try {
        static_cast<CompositionBlurContext*>(context)->set_blur(blur_amount);
        return 1;
    } catch (...) {
        return 0;
    }
}

extern "C" __declspec(dllexport) int codex_composition_blur_set_tint(
    void* context,
    unsigned char r,
    unsigned char g,
    unsigned char b,
    unsigned char a) noexcept
{
    if (!context) {
        return 0;
    }
    try {
        static_cast<CompositionBlurContext*>(context)->set_tint(r, g, b, a);
        return 1;
    } catch (...) {
        return 0;
    }
}

extern "C" __declspec(dllexport) void codex_composition_blur_destroy(void* context) noexcept
{
    delete static_cast<CompositionBlurContext*>(context);
}
