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
        return -1;
    }

    HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    const bool uninitialize = SUCCEEDED(init);
    if (FAILED(init) && init != RPC_E_CHANGED_MODE) {
        return -1;
    }

    IUIAutomation* automation = nullptr;
    IUIAutomationElement* root = nullptr;
    IUIAutomationCondition* condition = nullptr;
    IUIAutomationElementArray* elements = nullptr;
    int written = -1;
    int length = 0;

    HRESULT hr = CoCreateInstance(
        CLSID_CUIAutomation,
        nullptr,
        CLSCTX_INPROC_SERVER,
        IID_PPV_ARGS(&automation)
    );
    if (FAILED(hr) || !automation) {
        goto cleanup;
    }

    hr = automation->ElementFromHandle(taskbar_hwnd, &root);
    if (FAILED(hr) || !root) {
        goto cleanup;
    }

    hr = automation->CreateTrueCondition(&condition);
    if (FAILED(hr) || !condition) {
        goto cleanup;
    }

    hr = root->FindAll(TreeScope_Descendants, condition, &elements);
    if (FAILED(hr) || !elements) {
        goto cleanup;
    }

    hr = elements->get_Length(&length);
    if (FAILED(hr)) {
        goto cleanup;
    }

    if (length < 0 || static_cast<std::size_t>(length) > capacity) {
        goto cleanup;
    }

    written = 0;
    for (int index = 0; index < length; ++index) {
        IUIAutomationElement* element = nullptr;
        if (FAILED(elements->GetElement(index, &element)) || !element) {
            written = -1;
            goto cleanup;
        }

        BOOL offscreen = TRUE;
        CONTROLTYPEID control_type = 0;
        RECT rect{};
        const HRESULT offscreen_hr = element->get_CurrentIsOffscreen(&offscreen);
        const HRESULT type_hr = element->get_CurrentControlType(&control_type);
        const HRESULT rect_hr = element->get_CurrentBoundingRectangle(&rect);

        if (FAILED(offscreen_hr) || FAILED(type_hr) || FAILED(rect_hr)) {
            element->Release();
            written = -1;
            goto cleanup;
        }

        if (!offscreen
            && is_actionable_taskbar_control(control_type)
            && rect.right > rect.left
            && rect.bottom > rect.top) {
            out_rects[written++] = CodexTaskbarControlRect{
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
            };
        }

        element->Release();
    }

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
    return written;
}
