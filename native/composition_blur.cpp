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
    wuc::InsetClip root_clip{nullptr};
    wuc::SpriteVisual blur_visual{nullptr};
    wuc::SpriteVisual tint_visual{nullptr};
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
        root_clip = compositor.CreateInsetClip();
        root.Clip(root_clip);

        blur_visual = compositor.CreateSpriteVisual();
        blur_visual.Brush(blur_brush);
        root.Children().InsertAtBottom(blur_visual);

        tint_brush = compositor.CreateColorBrush(winrt::Windows::UI::Color{a, r, g, b});
        tint_visual = compositor.CreateSpriteVisual();
        tint_visual.Brush(tint_brush);
        root.Children().InsertAtTop(tint_visual);

        // Do not rely on DesktopWindowTarget's asynchronous resize propagation.
        // Keep the entire visual tree explicitly bounded and clipped.
        set_bounds(1.0f, 1.0f);
        target.Root(root);
    }

    void set_bounds(float width, float height)
    {
        const float safe_width = width > 1.0f ? width : 1.0f;
        const float safe_height = height > 1.0f ? height : 1.0f;
        const winrt::Windows::Foundation::Numerics::float2 size{
            safe_width,
            safe_height,
        };
        root.Size(size);
        blur_visual.Size(size);
        tint_visual.Size(size);
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
    float height) noexcept
{
    if (!context || width <= 0.0f || height <= 0.0f) {
        return 0;
    }
    try {
        static_cast<CompositionBlurContext*>(context)->set_bounds(width, height);
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
