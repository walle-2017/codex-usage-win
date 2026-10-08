#include <windows.h>
#include <uiautomation.h>

#include <cstddef>
#include <cstdint>

struct CodexTaskbarControlRect {
    int left;
    int top;
    int right;
    int bottom;
};

static bool is_actionable_taskbar_control(CONTROLTYPEID type) {
    switch (type) {
        case UIA_ButtonControlTypeId:
        case UIA_CheckBoxControlTypeId:
        case UIA_ComboBoxControlTypeId:
        case UIA_EditControlTypeId:
        case UIA_HyperlinkControlTypeId:
        case UIA_ListItemControlTypeId:
        case UIA_MenuItemControlTypeId:
        case UIA_RadioButtonControlTypeId:
        case UIA_SplitButtonControlTypeId:
        case UIA_TabItemControlTypeId:
        case UIA_ThumbControlTypeId:
        case UIA_TreeItemControlTypeId:
            return true;
        default:
            return false;
    }
}

extern "C" __declspec(dllexport) int codex_taskbar_control_rects(
    std::intptr_t taskbar_hwnd_raw,
    CodexTaskbarControlRect* out_rects,
    std::size_t capacity
) {
    HWND taskbar_hwnd = reinterpret_cast<HWND>(taskbar_hwnd_raw);
    if (!taskbar_hwnd || !IsWindow(taskbar_hwnd) || !out_rects || capacity == 0) {
        return static_cast<int>(E_INVALIDARG);
    }

    HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    const bool uninitialize = SUCCEEDED(init);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) {
        return static_cast<int>(init);
    }

    IUIAutomation* automation = nullptr;
    IUIAutomationElement* root = nullptr;
    IUIAutomationCondition* condition = nullptr;
    IUIAutomationElementArray* elements = nullptr;
    int result = static_cast<int>(E_FAIL);
    int written = 0;
    int length = 0;

    HRESULT hr = CoCreateInstance(
        CLSID_CUIAutomation,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(&automation)
    );
    if (FAILED(hr) || !automation) {
        result = FAILED(hr) ? static_cast<int>(hr) : static_cast<int>(E_UNEXPECTED);
        goto cleanup;
    }

    hr = automation->ElementFromHandle(taskbar_hwnd, &root);
    if (FAILED(hr) || !root) {
        result = FAILED(hr) ? static_cast<int>(hr) : static_cast<int>(E_UNEXPECTED);
        goto cleanup;
    }

    hr = automation->CreateTrueCondition(&condition);
    if (FAILED(hr) || !condition) {
        result = FAILED(hr) ? static_cast<int>(hr) : static_cast<int>(E_UNEXPECTED);
        goto cleanup;
    }

    hr = root->FindAll(TreeScope_Descendants, condition, &elements);
    if (FAILED(hr) || !elements) {
        result = FAILED(hr) ? static_cast<int>(hr) : static_cast<int>(E_UNEXPECTED);
        goto cleanup;
    }

    hr = elements->get_Length(&length);
    if (FAILED(hr)) {
        result = static_cast<int>(hr);
        goto cleanup;
    }
    if (length < 0) {
        result = static_cast<int>(E_UNEXPECTED);
        goto cleanup;
    }

    for (int index = 0; index < length; ++index) {
        IUIAutomationElement* element = nullptr;
        hr = elements->GetElement(index, &element);
        if (FAILED(hr) || !element) {
            result = FAILED(hr) ? static_cast<int>(hr) : static_cast<int>(E_UNEXPECTED);
            goto cleanup;
        }

        CONTROLTYPEID control_type = 0;
        hr = element->get_CurrentControlType(&control_type);
        if (FAILED(hr)) {
            element->Release();
            result = static_cast<int>(hr);
            goto cleanup;
        }
        if (!is_actionable_taskbar_control(control_type)) {
            element->Release();
            continue;
        }

        BOOL offscreen = TRUE;
        RECT rect{};
        hr = element->get_CurrentIsOffscreen(&offscreen);
        if (FAILED(hr)) {
            element->Release();
            result = static_cast<int>(hr);
            goto cleanup;
        }
        hr = element->get_CurrentBoundingRectangle(&rect);
        if (FAILED(hr)) {
            element->Release();
            result = static_cast<int>(hr);
            goto cleanup;
        }

        if (!offscreen
            && rect.right > rect.left
            && rect.bottom > rect.top) {
            if (static_cast<std::size_t>(written) >= capacity) {
                element->Release();
                result = static_cast<int>(HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER));
                goto cleanup;
            }
            out_rects[written++] = CodexTaskbarControlRect{
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
            };
        }

        element->Release();
    }

    result = written;

cleanup:
    if (elements) {
        elements->Release();
    }
    if (condition) {
        condition->Release();
    }
    if (root) {
        root->Release();
    }
    if (automation) {
        automation->Release();
    }
    if (uninitialize) {
        CoUninitialize();
    }
    return result;
}
