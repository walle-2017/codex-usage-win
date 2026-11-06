//! Stable Win32 color-channel packing and numeric color-target indices.
//! Preserve the public decode/unpack API and existing channel bit order.
use super::*;

pub fn decode_color_target(value: usize) -> Option<StyleColorTarget> {
    Some(match value {
        0 => StyleColorTarget::PanelBackground,
        1 => StyleColorTarget::PanelBorder,
        2 => StyleColorTarget::QuotaType,
        3 => StyleColorTarget::Remaining,
        4 => StyleColorTarget::ResetTime,
        5 => StyleColorTarget::Error,
        6 => StyleColorTarget::ProgressHigh,
        7 => StyleColorTarget::ProgressMedium,
        8 => StyleColorTarget::ProgressLow,
        9 => StyleColorTarget::ProgressConsumed,
        10 => StyleColorTarget::DragHandle,
        11 => StyleColorTarget::TooltipBackground,
        12 => StyleColorTarget::TooltipBorder,
        _ => return None,
    })
}

pub fn unpack_color(value: isize) -> Color {
    let value = value as u32;
    Color::rgba(
        (value & 0xFF) as u8,
        ((value >> 8) & 0xFF) as u8,
        ((value >> 16) & 0xFF) as u8,
        ((value >> 24) & 0xFF) as u8,
    )
}

pub(super) fn encode_color_target(target: StyleColorTarget) -> usize {
    match target {
        StyleColorTarget::PanelBackground => 0,
        StyleColorTarget::PanelBorder => 1,
        StyleColorTarget::QuotaType => 2,
        StyleColorTarget::Remaining => 3,
        StyleColorTarget::ResetTime => 4,
        StyleColorTarget::Error => 5,
        StyleColorTarget::ProgressHigh => 6,
        StyleColorTarget::ProgressMedium => 7,
        StyleColorTarget::ProgressLow => 8,
        StyleColorTarget::ProgressConsumed => 9,
        StyleColorTarget::DragHandle => 10,
        StyleColorTarget::TooltipBackground => 11,
        StyleColorTarget::TooltipBorder => 12,
    }
}

pub(super) fn pack_color(color: Color) -> isize {
    (u32::from(color.r)
        | (u32::from(color.g) << 8)
        | (u32::from(color.b) << 16)
        | (u32::from(color.a) << 24)) as isize
}

pub(super) fn parse_hex_input(value: &str) -> Result<Option<Color>, ()> {
    let trimmed = value.trim();
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if digits.len() > 8 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(());
    }
    match digits.len() {
        6 | 8 => Color::try_from_hex(trimmed).map(Some).ok_or(()),
        0..=5 | 7 => Ok(None),
        _ => Err(()),
    }
}

pub(super) fn target_from_hex_control_id(control_id: u16) -> Option<StyleColorTarget> {
    let index = control_id.checked_sub(ID_EDIT_HEX_BASE)? as usize;
    COLOR_TARGETS.get(index).copied()
}
