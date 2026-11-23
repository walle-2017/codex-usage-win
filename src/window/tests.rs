//! Windows taskbar and floating widget geometry / regression tests.
//! Kept in a child module to preserve access to the window state and helpers.

use super::tooltip_measure::{tooltip_mask_content_width, tooltip_width_with_render_slack};
use crate::popup_menu::PopupAction;
#[test]
fn quick_appearance_menu_tracks_explicit_user_choices() {
    let [appearance, desktop] = quick_appearance_submenus(
        true, ThemeMode::System, AppearancePreset::Minimal,
        DisplayPosition::Taskbar, DesktopDirection::Vertical, false,
    );
    let PopupAction::Submenu(choices) = appearance.action else { panic!("appearance submenu expected") };
    let checked: Vec<u16> = choices.iter().filter(|item| item.checked).filter_map(|item| {
        if let PopupAction::Command(id) = &item.action { Some(*id) } else { None }
    }).collect();
    assert_eq!(checked, vec![IDM_THEME_SYSTEM, IDM_LAYOUT_MINIMAL, IDM_QUICK_POSITION_TASKBAR]);
    assert!(!desktop.enabled);
    let PopupAction::Submenu(desktop_choices) = desktop.action else { panic!("desktop submenu expected") };
    assert!(desktop_choices[1].checked);
    assert!(desktop_choices[4].checked);
}

#[test]
fn desktop_shortcuts_activate_only_for_explicit_desktop_position() {
    let [_, desktop] = quick_appearance_submenus(
        false, ThemeMode::Dark, AppearancePreset::Default,
        DisplayPosition::Desktop, DesktopDirection::Horizontal, true,
    );
    assert!(desktop.enabled);
    let PopupAction::Submenu(choices) = desktop.action else { panic!("desktop submenu expected") };
    assert!(choices[0].checked);
    assert!(choices[3].checked);
    assert!(choices.iter().filter(|item| item.checked).count() == 2);
}

use super::*;

// Geometry adapters retained solely for regression tests of historical
// edge cases; production uses the unified hover and handle render paths.
/// Rounded-rectangle signed distance. Negative is inside the actual panel,
/// positive is outside; returns zero at its edge.
fn drag_hover_signed_edge_distance(width: i32, height: i32, x: i32, y: i32, radius: i32) -> f32 {
    let inset = sc(1).max(1) as f32;
    let half_w = (width as f32 / 2.0 - inset).max(0.5);
    let half_h = (height as f32 / 2.0 - inset).max(0.5);
    let radius = (radius as f32).min(half_w.min(half_h)).max(0.0);
    let dx = (x as f32 + 0.5 - width as f32 / 2.0).abs() - (half_w - radius);
    let dy = (y as f32 + 0.5 - height as f32 / 2.0).abs() - (half_h - radius);
    dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - radius
}

fn draw_drag_hover_outline(
    pixels: &mut [u32], canvas_w: i32, canvas_h: i32,
    bounds: RECT, corner_radius: u8, color: Color,
) {
    if canvas_w <= 0 || canvas_h <= 0 || bounds.right <= bounds.left
        || bounds.bottom <= bounds.top { return; }
    let w = bounds.right - bounds.left;
    let h = bounds.bottom - bounds.top;
    let radius = sc(i32::from(corner_radius))
        .min((w.min(h) / 2).max(0));
    let extent = sc(12).max(2);
    // Restrict work to the edge envelope, avoiding per-pixel work over the
    // center of larger horizontal panels.
    for y in bounds.top.saturating_sub(extent).max(0)
        ..bounds.bottom.saturating_add(extent).min(canvas_h) {
        for x in bounds.left.saturating_sub(extent).max(0)
            ..bounds.right.saturating_add(extent).min(canvas_w) {
            let local_x = x - bounds.left;
            let local_y = y - bounds.top;
            let distance = drag_hover_signed_edge_distance(w, h, local_x, local_y, radius);
            if distance.abs() > extent as f32 { continue; }
            let coverage = drag_hover_glow_coverage(distance);
            if coverage == 0 { continue; }
            let index = (y * canvas_w + x) as usize;
            pixels[index] = composite_premultiplied_text(pixels[index], color, coverage);
        }
    }
}

/// Draw the dedicated desktop drag tab into the premultiplied layered surface.
/// When collapsed, every pixel outside the tab is fully transparent and cannot
/// obstruct other desktop windows or leave a visible panel edge.
fn render_floating_handle_bitmap(
    pixels: &mut [u32], width: i32, height: i32,
    edge: Option<FloatingEdge>, hovered: bool, style: &ThemeStyle, opacity: u8,
) {
    let grip = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
    render_floating_handle_bitmap_at(pixels, width, height, grip, hovered, style, opacity);
}

fn floating_drag_handle_hit(
    x: i32, y: i32, width: i32, height: i32, edge: Option<FloatingEdge>, peek: i32,
) -> bool {
    point_in_rect(floating_grip_rect(width, height, edge, peek), x, y)
}

// The desktop tab lives OUTSIDE the panel body. Hover must be checked against
// its final screen-space rectangle, especially after the canvas origin changes
// on drop from a docked edge to the free-floating top tab.
fn floating_drag_grip_contains_screen_point(
    origin_x: i32, origin_y: i32, canvas_w: i32, canvas_h: i32,
    edge: Option<FloatingEdge>, pointer: POINT,
) -> bool {
    floating_drag_handle_hit(
        pointer.x - origin_x, pointer.y - origin_y,
        canvas_w, canvas_h, edge, sc(FLOAT_EDGE_PEEK),
    )
}

fn floating_clamp_free_visible_position(
    bounds: RECT, width: i32, height: i32, body: RECT, x: i32, y: i32,
) -> (i32, i32) {
    floating_clamp_drag_visible_position(bounds, width, height, body, None, false, x, y)
}


#[test]
fn v24_22_taskbar_inner_glow_never_touches_border_or_outside() {
    // Strictly no external pixels, no border recoloring, no center tint,
    // including rounded corners and fully transparent/opaque themes.
    for (width, height) in [(270, 53), (48, 168), (90, 24)] {
        for radius in [0_u8, 3, 10, 20] {
            for alpha in [0_u8, 1, 64, 255] {
                let mut style = ThemeStyle::preset(true, ThemePreset::Forest);
                style.panel_corner_radius = radius;
                style.set_color(StyleColorTarget::PanelBackground,
                    Color::rgba(18, 45, 37, alpha));
                let color = drag_hover_outline_color(&style);
                let original = vec![premultiplied_pixel(
                    style.color(StyleColorTarget::PanelBackground)
                ); (width * height) as usize];
                let mut pixels = original.clone();
                draw_taskbar_hover_inner_glow(
                    &mut pixels, width, height, &style, color,
                );
                let mut inside_changed = 0;
                let mut center_changed = 0;
                for y in 0..height {
                    for x in 0..width {
                        let idx = (y * width + x) as usize;
                        let inside = panel_inner_shape_contains(
                            width, height, x, y, radius,
                        );
                        if !inside {
                            assert_eq!(pixels[idx], original[idx],
                                "outside and border pixels must be unchanged");
                        } else if pixels[idx] != original[idx] {
                            inside_changed += 1;
                            if x == width / 2 && y == height / 2 {
                                center_changed += 1;
                            }
                        }
                        // Layered bitmap must remain premultiplied BGRA.
                        let a = pixels[idx] >> 24;
                        for shift in [0, 8, 16] {
                            assert!(((pixels[idx] >> shift) & 255) <= a);
                        }
                    }
                }
                assert!(inside_changed > 0,
                    "every supported shape must show an inward halo");
                assert_eq!(center_changed, 0,
                    "far-center quota content must not be tinted");
            }
        }
    }
}

#[test]
fn v24_22_taskbar_inner_glow_contacts_border_and_fades_inward() {
    let width = 270;
    let height = 53;
    let mut style = ThemeStyle::preset(true, ThemePreset::Forest);
    style.panel_corner_radius = 10;
    let mut pixels = vec![0u32; (width * height) as usize];
    draw_taskbar_hover_inner_glow(
        &mut pixels, width, height, &style, drag_hover_outline_color(&style),
    );
    let top = sc(1).max(1) + PANEL_BORDER_WIDTH_PX;
    let x = width / 2;
    let alpha = |y: i32| pixels[(y * width + x) as usize] >> 24;
    assert_eq!(alpha(top - 1), 0, "painted border is unchanged");
    assert!(alpha(top) > alpha(top + 2),
        "the glow must start against the inner border and fall inward");
    assert!(alpha(top + 2) > alpha(top + 4));
    assert_eq!(alpha(height / 2), 0, "center stays untouched");
    // Desktop uses a completely separate exterior painter. Both can be
    // invoked independently without sharing state or mask buffers.
    let canvas_w = width + sc(24);
    let canvas_h = height + sc(24);
    let margin = sc(12);
    let mut desktop = vec![0u32; (canvas_w * canvas_h) as usize];
    draw_drag_hover_outer_glow(
        &mut desktop, canvas_w, canvas_h,
        RECT { left: margin, top: margin, right: margin + width,
            bottom: margin + height },
        RECT::default(), &style, drag_hover_outline_color(&style),
    );
    assert!(desktop[((margin - 1) * canvas_w + margin + x) as usize] >> 24 > 0,
        "existing desktop exterior halo must still render");
}

#[test]
fn premultiplied_text_composition_preserves_partial_coverage() {
    let panel = premultiplied_pixel(Color::rgba(240, 240, 240, 128));
    let mixed = composite_premultiplied_text(
        panel,
        Color::rgba(32, 32, 32, 255),
        128,
    );
    let alpha = (mixed >> 24) & 0xFF;
    assert!(alpha > 128 && alpha < 255);
}

#[test]
fn frosted_strength_maps_linearly_to_gaussian_radius() {
    assert_eq!(blur_amount_for_strength(0), 0.0);
    assert!((blur_amount_for_strength(1) - 0.2).abs() < f32::EPSILON);
    assert!((blur_amount_for_strength(10) - 2.0).abs() < f32::EPSILON);
    assert!((blur_amount_for_strength(50) - 10.0).abs() < f32::EPSILON);
    assert!((blur_amount_for_strength(100) - 20.0).abs() < f32::EPSILON);
    assert!((blur_amount_for_strength(255) - 20.0).abs() < f32::EPSILON);
}

#[test]
fn point_in_taskbar_rect_uses_half_open_bounds() {
    let rect = RECT {
        left: 100,
        top: 200,
        right: 500,
        bottom: 260,
    };
    assert!(point_is_in_taskbar_rect(POINT { x: 100, y: 200 }, rect));
    assert!(point_is_in_taskbar_rect(POINT { x: 499, y: 259 }, rect));
    assert!(!point_is_in_taskbar_rect(POINT { x: 500, y: 259 }, rect));
    assert!(!point_is_in_taskbar_rect(POINT { x: 499, y: 260 }, rect));
}

#[test]
fn transient_taskbar_enumeration_gap_keeps_valid_cached_handle_active() {
    let cache = vec![TaskbarLayoutSnapshot {
        taskbar_hwnd_raw: 11,
        controls: vec![RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 48,
        }],
    }];
    let mut active = BTreeSet::new();

    preserve_valid_cached_taskbars(&mut active, &cache, |raw| raw == 11);

    assert!(active.contains(&11));
}

#[test]
fn invalid_cached_handle_is_not_kept_active() {
    let cache = vec![TaskbarLayoutSnapshot {
        taskbar_hwnd_raw: 11,
        controls: vec![RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 48,
        }],
    }];
    let mut active = BTreeSet::new();

    preserve_valid_cached_taskbars(&mut active, &cache, |_| false);

    assert!(!active.contains(&11));
}

#[test]
fn transient_taskbar_query_failure_keeps_last_known_good_cache() {
    let original = RECT {
        left: 100,
        top: 0,
        right: 200,
        bottom: 48,
    };
    let mut cache = vec![TaskbarLayoutSnapshot {
        taskbar_hwnd_raw: 11,
        controls: vec![original],
    }];
    let active = BTreeSet::from([11isize]);

    let changed = merge_taskbar_layout_cache(&mut cache, &active, Vec::new());

    assert!(!changed);
    assert_eq!(cache.len(), 1);
    assert!(same_control_rects(&cache[0].controls, &[original]));
}

#[test]
fn successful_taskbar_query_replaces_last_known_good_cache() {
    let original = RECT {
        left: 100,
        top: 0,
        right: 200,
        bottom: 48,
    };
    let updated = RECT {
        left: 120,
        top: 0,
        right: 220,
        bottom: 48,
    };
    let mut cache = vec![TaskbarLayoutSnapshot {
        taskbar_hwnd_raw: 11,
        controls: vec![original],
    }];
    let active = BTreeSet::from([11isize]);

    let changed = merge_taskbar_layout_cache(
        &mut cache,
        &active,
        vec![TaskbarLayoutSnapshot {
            taskbar_hwnd_raw: 11,
            controls: vec![updated],
        }],
    );

    assert!(changed);
    assert!(same_control_rects(&cache[0].controls, &[updated]));
}

#[test]
fn disappeared_taskbar_is_removed_from_layout_cache() {
    let mut cache = vec![TaskbarLayoutSnapshot {
        taskbar_hwnd_raw: 11,
        controls: vec![RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 48,
        }],
    }];
    let active = BTreeSet::new();

    let changed = merge_taskbar_layout_cache(&mut cache, &active, Vec::new());

    assert!(changed);
    assert!(cache.is_empty());
}

#[test]
fn left_and_right_taskbars_select_vertical_axis() {
    let left = RECT { left: 0, top: 0, right: 48, bottom: 1080 };
    let right = RECT { left: 1920, top: -100, right: 1980, bottom: 980 };
    let horizontal = RECT { left: -1920, top: 1040, right: 0, bottom: 1080 };
    assert_eq!(taskbar_axis(left), TaskbarAxis::Vertical);
    assert_eq!(taskbar_axis(right), TaskbarAxis::Vertical);
    assert_eq!(taskbar_axis(horizontal), TaskbarAxis::Horizontal);
    assert!(vertical_widget_width(left).is_some());
    assert!(vertical_widget_width(right).is_some());
    assert_eq!(vertical_widget_width(RECT { right: 28, ..left }), None);
}

#[test]
fn vertical_taskbar_free_gaps_exclude_pinned_icons_and_tray() {
    let taskbar = RECT { left: 1880, top: -100, right: 1928, bottom: 980 };
    let controls = [
        RECT { left: 1880, top: -100, right: 1928, bottom: 20 },
        RECT { left: 1880, top: 120, right: 1928, bottom: 340 },
        RECT { left: 1880, top: 860, right: 1928, bottom: 980 },
        // A window outside the taskbar's horizontal footprint is not an obstacle.
        RECT { left: 0, top: 400, right: 60, bottom: 500 },
    ];
    let gaps = free_vertical_spans_from_occupied(taskbar, 860, &controls, 4);
    assert_eq!(gaps, vec![
        VerticalSpan { top: 24, bottom: 116 },
        VerticalSpan { top: 344, bottom: 856 },
    ]);
    assert_eq!(nearest_free_top_offset(taskbar, &gaps, 104, -100), Some(444));
    assert_eq!(nearest_free_top_offset(taskbar, &gaps, 104, 900), Some(852));
}

#[test]
fn vertical_ring_has_correct_clockwise_progress_and_clear_center() {
    let width = 48;
    let height = 128;
    let mut pixels = vec![0u32; (width * height) as usize];
    let track = Color::rgba(70, 70, 70, 255);
    let accent = Color::rgba(0, 230, 0, 255);
    render_vertical_ring(
        &mut pixels, width, height, 24, 40, 15.0, 25.0,
        track, accent, true,
    );
    let pixel = |x: i32, y: i32| pixels[(y * width + x) as usize];
    // The bottom opening stays transparent; 25% begins at the lower left
    // and advances clockwise to the left rather than representing usage.
    assert_eq!(pixel(24, 55), 0);
    assert!((pixel(9, 40) & 0x0000FF00) > (pixel(24, 25) & 0x0000FF00));
    assert_eq!(pixel(24, 40), 0);
    assert_ne!(pixel(24, 25), 0);
    assert!(vertical_ring_fraction(0.0).is_some());
    assert_eq!(vertical_ring_fraction(std::f32::consts::PI), None);
    let mut complete_pixels = vec![0u32; (width * height) as usize];
    render_vertical_ring(
        &mut complete_pixels, width, height, 24, 40, 15.0, 100.0,
        track, accent, true,
    );
    assert_eq!(complete_pixels[(55 * width + 24) as usize], 0);
    assert_ne!(complete_pixels[(25 * width + 24) as usize], 0);
    assert_eq!(vertical_quota_text(2.0, true), "2%");
    assert_eq!(vertical_quota_text(100.0, true), "100%");
}

#[test]
fn vertical_layout_keeps_both_reset_lines_inside_compact_height() {
    let height = sc(VERTICAL_WIDGET_HEIGHT);
    let (session, weekly) = vertical_row_tops(height, true, true);
    let first = session.unwrap();
    let second = weekly.unwrap();
    // The handle, text, ring, reset time and next row must be disjoint.
    assert!(first >= sc(VERTICAL_DRAG_HANDLE_H));
    assert!(second - first >= sc(VERTICAL_QUOTA_ROW_HEIGHT + VERTICAL_QUOTA_ROW_GAP));
    let outer_radius = sc(VERTICAL_RING_MAX_RADIUS) + sc(2);
    assert!(sc(VERTICAL_LABEL_CENTER_Y + 9) < sc(VERTICAL_RING_CENTER_Y) - outer_radius);
    assert!(sc(VERTICAL_RING_CENTER_Y) + outer_radius < sc(VERTICAL_RESET_CENTER_Y - 9));
    assert!(second + sc(VERTICAL_RESET_CENTER_Y + 9) <= height - sc(2));
    assert!(vertical_drag_handle_hit(16, sc(8), sc(32), height));
    assert!(!vertical_drag_handle_hit(16, first, sc(32), height));
    assert!(!vertical_drag_handle_hit(16, second, sc(32), height));
    assert!(!vertical_drag_handle_hit(16, sc(18), sc(32), height));
    assert_eq!(vertical_reset_label(None, false), "--");
    assert_eq!(vertical_reset_label(None, true), "--");
}

#[test]
fn desktop_vertical_panel_height_tracks_visible_rows_and_minimal_preset() {
    for preset in [AppearancePreset::Default, AppearancePreset::Minimal] {
        let one = desktop_vertical_content_height(preset, true, false);
        let weekly_only = desktop_vertical_content_height(preset, false, true);
        let both = desktop_vertical_content_height(preset, true, true);
        assert_eq!(one, weekly_only);
        assert!(both > one);
        assert!(both < vertical_widget_height_for_preset(preset));
        let inset = sc(DESKTOP_PANEL_CONTENT_PADDING);
        if preset == AppearancePreset::Minimal {
            for (height, session, weekly) in [
                (one, true, false), (one, false, true), (both, true, true),
            ] {
                let (a, b) = vertical_minimal_percent_centers_for_layout(
                    height, session, weekly, true,
                );
                for center in [a, b].into_iter().flatten() {
                    assert!(center - sc(12) >= inset - sc(1));
                    assert!(center + sc(12) <= height - inset + sc(1));
                }
            }
        } else {
            for (height, session, weekly) in [
                (one, true, false), (one, false, true), (both, true, true),
            ] {
                let (a, b) = vertical_row_tops_for_layout(
                    height, session, weekly, true,
                );
                let first_visible = a.or(b).unwrap();
                assert_eq!(first_visible, inset,
                    "first visible row must begin at the top panel inset");
                if let (Some(first), Some(second)) = (a, b) {
                    assert_eq!(second - first,
                        sc(VERTICAL_QUOTA_ROW_HEIGHT + VERTICAL_QUOTA_ROW_GAP));
                }
            }
            let (_, weekly) = vertical_row_tops_for_layout(both, true, true, true);
            let last_reset_bottom =
                weekly.unwrap() + sc(VERTICAL_RESET_CENTER_Y + 9);
            assert!(last_reset_bottom <= both - inset + sc(1));
        }
    }
    assert_eq!(
        vertical_widget_height_for_preset(AppearancePreset::Default),
        sc(VERTICAL_WIDGET_HEIGHT),
        "embedded taskbar's legacy height must stay unchanged",
    );
}

#[test]
fn desktop_horizontal_width_tracks_active_usage_text_without_embedded_drag_strip() {
    let lang = LanguageId::English;
    let normal = AppearancePreset::Default;
    let both_resets = desktop_horizontal_content_width(
        lang, normal, true, true, "70%  15:05", "88%  10/25",
    );
    let one_reset = desktop_horizontal_content_width(
        lang, normal, true, false, "70%  15:05", "88%",
    );
    assert_eq!(both_resets, one_reset, "rows share one aligned width");
    assert_eq!(
        both_resets + sc(DRAG_HANDLE_HIT_W)
            - 2 * (sc(DESKTOP_PANEL_CONTENT_PADDING)
                - sc(normal.metrics().outer_padding)),
        total_widget_width_for_preset(lang, normal),
        "desktop replaces the taskbar drag strip with equal larger content insets",
    );
    let no_reset = desktop_horizontal_content_width(
        lang, normal, true, true, "70%", "88%",
    );
    assert!(no_reset < both_resets, "absent reset labels free their whole column");
    let minimal = desktop_horizontal_content_width(
        lang, AppearancePreset::Minimal, true, true, "70%", "88%",
    );
    assert_eq!(
        minimal + sc(DRAG_HANDLE_HIT_W)
            - 2 * (sc(DESKTOP_PANEL_CONTENT_PADDING)
                - sc(AppearancePreset::Minimal.metrics().outer_padding)),
        total_widget_width_for_preset(lang, AppearancePreset::Minimal),
    );
    assert_eq!(horizontal_content_left(&normal.metrics(), true),
        sc(DESKTOP_PANEL_CONTENT_PADDING));
    assert_eq!(horizontal_content_left(&normal.metrics(), false),
        sc(normal.metrics().outer_padding + DRAG_HANDLE_HIT_W));
}

#[test]
fn desktop_visual_ink_alignment_avoids_reserved_reset_column_whitespace() {
    let width = 34;
    let height = 12;
    let mut mask = vec![0u8; (width * height) as usize];
    // Mimic a 5-character time mask with 4 pixels of leading bearing
    // and trailing blank columns from the fixed-width text rectangle.
    for y in 2..10 {
        for x in 4..27 {
            mask[(y * width + x) as usize] = 255;
        }
    }
    assert_eq!(directwrite_mask_x_offset(
        &mask, width, height, HorizontalTextAlign::Start,
    ), 0, "embedded taskbar keeps its original layout");
    let shift_start = directwrite_mask_x_offset(
        &mask, width, height, HorizontalTextAlign::InkStart,
    );
    assert_eq!(4 + shift_start, 0,
        "desktop first visible glyph must align to the left content inset");
    let shift_end = directwrite_mask_x_offset(
        &mask, width, height, HorizontalTextAlign::End,
    );
    assert_eq!(26 + shift_end, width - 1,
        "reset text must reach the content inset, not stop at its reserved width");
    let shift_center = directwrite_mask_x_offset(
        &mask, width, height, HorizontalTextAlign::Center,
    );
    assert!(((4 + shift_center) - (width - 1 - (26 + shift_center))).abs() <= 1,
        "the minimal single value must be optically centered");
    assert_eq!(directwrite_mask_x_offset(
        &vec![0; (width * height) as usize],
        width, height, HorizontalTextAlign::End,
    ), 0);
}

#[test]
fn desktop_horizontal_and_vertical_content_margins_are_symmetric() {
    let preset = AppearancePreset::Default;
    let metrics = preset.metrics();
    let left = horizontal_content_left(&metrics, true);
    let width = desktop_horizontal_content_width(
        LanguageId::English, preset, true, true, "74%  15:05", "88%  10/25",
    );
    let label = sc(metrics.label_width);
    let bar = row_bar_segment_count(preset)
        * (sc(SEGMENT_W) + sc(SEGMENT_GAP)) - sc(SEGMENT_GAP);
    let rightmost_text_edge = left + label + sc(metrics.label_bar_gap)
        + bar + sc(metrics.bar_percent_gap)
        + sc(metrics.percent_width) + sc(metrics.percent_reset_gap)
        + sc(metrics.reset_width);
    assert_eq!(left, sc(DESKTOP_PANEL_CONTENT_PADDING));
    assert_eq!(width - rightmost_text_edge, left,
        "left and right gutters must match around text ink");
    for p in [AppearancePreset::Default, AppearancePreset::Minimal] {
        for (show_session, show_weekly) in [(true, true), (true, false), (false, true)] {
            let h = desktop_vertical_content_height(p, show_session, show_weekly);
            let row_height = if p == AppearancePreset::Minimal {
                sc(DESKTOP_MINIMAL_ROW_HEIGHT)
            } else {
                sc(VERTICAL_QUOTA_ROW_HEIGHT)
            };
            let (first, last) = if p == AppearancePreset::Minimal {
                let (a, b) = vertical_minimal_percent_centers_for_layout(
                    h, show_session, show_weekly, true,
                );
                let a = a.or(b).unwrap() - row_height / 2;
                let b = b.unwrap_or(a + row_height / 2) + row_height / 2;
                (a, b)
            } else {
                let (a, b) = vertical_row_tops_for_layout(
                    h, show_session, show_weekly, true,
                );
                let first = a.or(b).unwrap();
                let last = b.or(a).unwrap() + row_height;
                (first, last)
            };
            assert!((first - (h - last)).abs() <= 1,
                "vertical visible row envelope must have equal top/bottom padding");
        }
    }
    assert_eq!(horizontal_content_left(&metrics, false),
        sc(metrics.outer_padding + DRAG_HANDLE_HIT_W),
        "taskbar mode must preserve its original leading drag strip");
}

#[test]
fn desktop_responsive_tooltip_uses_content_relative_coordinates() {
    let width = desktop_horizontal_content_width(
        LanguageId::English, AppearancePreset::Minimal,
        true, false, "70%", "88%",
    );
    let height = sc(AppearancePreset::Minimal.metrics().widget_height);
    let hit = desktop_horizontal_usage_hover_hit(
        width, height, AppearancePreset::Minimal, true, false,
        sc(10), height / 2,
    );
    assert_eq!(hit.map(|(target, _)| target), Some(MinimalHoverTarget::Session));
    assert_eq!(desktop_horizontal_usage_hover_hit(
        width, height, AppearancePreset::Minimal, true, false,
        width + 1, height / 2,
    ), None);
}

#[test]
fn v24_3_drag_release_preserves_hover_on_detached_tab_without_mouse_motion() {
    let edges = [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
        Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)];
    for (body_w, body_h) in [(240, 42), (48, 168)] {
        for edge in edges {
            let (width, height, body) =
                floating_canvas_geometry(body_w, body_h, edge);
            let tab = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
            let origin = POINT { x: -500, y: 320 };
            let pointer = POINT {
                x: origin.x + (tab.left + tab.right) / 2,
                y: origin.y + (tab.top + tab.bottom) / 2,
            };
            assert!(floating_drag_grip_contains_screen_point(
                origin.x, origin.y, width, height, edge, pointer,
            ));
            // The grip is independent of the body. The previous
            // body-based check cleared hover despite a stationary cursor.
            assert!(!point_in_rect(body,
                pointer.x - origin.x, pointer.y - origin.y));
            let outside = POINT { x: origin.x + body.left + 2,
                y: origin.y + body.top + body_h / 2 };
            assert!(!floating_drag_grip_contains_screen_point(
                origin.x, origin.y, width, height, edge, outside,
            ));
            assert!(!floating_drag_grip_contains_screen_point(
                origin.x, origin.y, width, height, edge,
                POINT { x: origin.x - 3, y: origin.y - 3 },
            ));
        }
    }
}

#[test]
fn v24_3_release_grip_hover_and_glow_use_the_same_frame() {
    let style = ThemeStyle::preset(true, ThemePreset::Ocean);
    let (width, height, body) = floating_canvas_geometry(240, 42, None);
    let tab = floating_grip_rect(width, height, None, sc(FLOAT_EDGE_PEEK));
    let pointer = POINT { x: 420 + (tab.left + tab.right) / 2,
        y: 180 + (tab.top + tab.bottom) / 2 };
    let hovered = floating_drag_grip_contains_screen_point(
        420, 180, width, height, None, pointer,
    );
    assert!(hovered);
    let alpha = if hovered { 100 } else { 0 };
    assert_eq!(alpha, 100, "tab must not fade on mouse release");
    let mut canvas = vec![0u32; (width * height) as usize];
    render_floating_handle_bitmap(
        &mut canvas, width, height, None, hovered, &style, alpha,
    );
    let grip_pixel = ((tab.top + tab.bottom) / 2 * width
        + (tab.left + tab.right) / 2) as usize;
    assert_ne!(canvas[grip_pixel], 0, "tab remains visible");
    let accent = drag_hover_outline_color(&style);
    draw_drag_hover_outline(
        &mut canvas, width, height, body, style.panel_corner_radius, accent,
    );
    let ring_pixel = ((body.top + body.bottom) / 2 * width + body.left - 1)
        as usize;
    assert_ne!(canvas[ring_pixel], 0, "body hover glow stays visible");
}

#[test]
fn v24_2_free_desktop_canvas_has_real_outer_halo_space() {
    for (width, height) in [(240, 42), (48, 168)] {
        let halo = sc(FLOAT_FREE_GLOW_MARGIN);
        let (cw, ch, body) = floating_canvas_geometry(width, height, None);
        let tab = floating_grip_rect(cw, ch, None, sc(FLOAT_EDGE_PEEK));
        assert_eq!(cw, width + halo * 2);
        assert_eq!(ch, height + sc(FLOAT_EDGE_PEEK) + halo * 2);
        assert_eq!(body.left, halo);
        assert_eq!(body.top, halo + sc(FLOAT_EDGE_PEEK));
        assert_eq!(cw - body.right, halo);
        assert_eq!(ch - body.bottom, halo);
        assert_eq!(tab.top, halo);
        assert_eq!(tab.bottom, body.top);
        assert!(floating_drag_handle_hit(
            (tab.left + tab.right) / 2, (tab.top + tab.bottom) / 2,
            cw, ch, None, sc(FLOAT_EDGE_PEEK),
        ));
        assert!(!floating_drag_handle_hit(
            0, tab.top, cw, ch, None, sc(FLOAT_EDGE_PEEK),
        ));
        let style = ThemeStyle::preset(true, ThemePreset::Forest);
        let mut bitmap = vec![0u32; (cw * ch) as usize];
        draw_drag_hover_outline(&mut bitmap, cw, ch, body,
            style.panel_corner_radius, drag_hover_outline_color(&style));
        // A genuine halo pixel exists left of the panel, and fades
        // completely before the outermost transparent canvas boundary.
        let y = (body.top + body.bottom) / 2;
        let alpha_at = |x: i32| bitmap[(y * cw + x) as usize] >> 24;
        assert!(alpha_at(body.left - 2) > alpha_at(body.left - 6));
        assert!(alpha_at(body.left - 6) > alpha_at(body.left - 10));
        assert_eq!(alpha_at(0), 0);
    }
    // All docked edges now have a transparent halo gutter on the three
    // inward-facing sides, while keeping the actual body at its monitor edge.
    let (w, h, dock) = floating_canvas_geometry(
        240, 42, Some(FloatingEdge::Bottom),
    );
    let glow = sc(FLOAT_FREE_GLOW_MARGIN);
    assert_eq!((w, h, dock.left, dock.top),
        (240 + glow * 2, 42 + sc(FLOAT_EDGE_PEEK) + glow,
         glow, sc(FLOAT_EDGE_PEEK) + glow));
}

#[test]
fn v24_6_hit_probe_has_no_extra_drag_targets_in_any_orientation() {
    for (bw, bh) in [(240, 42), (48, 168)] {
        for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
            Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
            let (w, h, body) = floating_canvas_geometry(bw, bh, edge);
            let grip = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
            let probe = floating_hover_probe_rect(body, grip, edge);
            let mut canvas = vec![0u32; (w * h) as usize];
            paint_floating_hover_hit_probe(&mut canvas, w, h, body, grip, edge);
            let grip_center = ((grip.left + grip.right) / 2,
                (grip.top + grip.bottom) / 2);
            assert_eq!(floating_drag_tab_hit_code(
                body, grip, grip_center.0, grip_center.1,
            ), 1, "rendered draggable tab must stay clickable");
            // The grab hotspot stays identical between actual dragging,
            // hover rendering and the non-client mouse hit classification.
            assert!(floating_drag_handle_hit(
                grip_center.0, grip_center.1, w, h, edge, sc(FLOAT_EDGE_PEEK),
            ));
            assert_eq!(canvas[(grip_center.1 * w + grip_center.0) as usize], 0,
                "hit-test probe must not paint over the real draggable tab");
            let body_center = ((body.left + body.right) / 2,
                (body.top + body.bottom) / 2);
            assert_eq!(floating_drag_tab_hit_code(
                body, grip, body_center.0, body_center.1,
            ), 1, "component panel remains interactive");
            for y in probe.top.max(0)..probe.bottom.min(h) {
                for x in probe.left.max(0)..probe.right.min(w) {
                    if point_in_rect(grip, x, y) || point_in_rect(body, x, y) {
                        continue;
                    }
                    assert_eq!(canvas[(y * w + x) as usize], 0x01000000,
                        "fully transparent hover probe must be alpha=1 only");
                    assert_eq!(floating_drag_tab_hit_code(body, grip, x, y), -1);
                    assert!(!floating_drag_handle_hit(
                        x, y, w, h, edge, sc(FLOAT_EDGE_PEEK),
                    ), "probe region must not allow drag start");
                }
            }
        }
    }
}

#[test]
fn v24_6_free_tab_lateral_exit_probe_extends_beyond_grip_width() {
    let (w, h, body) = floating_canvas_geometry(240, 42, None);
    let grip = floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK));
    let probe = floating_hover_probe_rect(body, grip, None);
    assert_eq!(probe.left, body.left);
    assert_eq!(probe.right, body.right);
    assert_eq!(probe.top, grip.top);
    assert_eq!(probe.bottom, body.top);
    let y = (grip.top + grip.bottom) / 2;
    let red_box_x = (body.left + grip.left) / 2;
    assert!(red_box_x < grip.left, "lateral point must be left of the tab");
    assert!(point_in_rect(probe, red_box_x, y));
    assert!(!floating_drag_handle_hit(
        red_box_x, y, w, h, None, sc(FLOAT_EDGE_PEEK),
    ));
    let mut pixels = vec![0u32; (w * h) as usize];
    paint_floating_hover_hit_probe(&mut pixels, w, h, body, grip, None);
    assert_eq!(pixels[(y * w + red_box_x) as usize], 0x01000000);
    assert_eq!(floating_drag_tab_hit_code(body, grip, red_box_x, y), -1);
}

#[test]
fn v24_7_drag_clears_physical_edge_and_restores_all_outer_gutters() {
    let g = sc(FLOAT_FREE_GLOW_MARGIN);
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    for (bw, bh) in [(240, 42), (48, 168)] {
        for edge in [FloatingEdge::Left, FloatingEdge::Right,
            FloatingEdge::Top, FloatingEdge::Bottom] {
            let (ow, oh, old) =
                floating_drag_canvas_geometry(bw, bh, Some(edge), false);
            let (nw, nh, new) =
                floating_drag_canvas_geometry(bw, bh, Some(edge), true);
            assert_eq!(new.right - new.left, bw);
            assert_eq!(new.bottom - new.top, bh);
            let shift_x = new.left - old.left;
            let shift_y = new.top - old.top;
            assert_eq!((nw - ow, nh - oh), match edge {
                FloatingEdge::Left | FloatingEdge::Right => (g, 0),
                FloatingEdge::Top | FloatingEdge::Bottom => (0, g),
            });
            assert_eq!((shift_x, shift_y), match edge {
                FloatingEdge::Left => (g, 0),
                FloatingEdge::Top => (0, g),
                _ => (0, 0),
            });
            // Keep both the panel and the grabbed tab in exactly the
            // same screen coordinates when transparent gutter is added.
            let old_tab = floating_grip_rect(ow, oh, Some(edge), sc(FLOAT_EDGE_PEEK));
            let new_tab = floating_grip_rect(nw, nh, Some(edge), sc(FLOAT_EDGE_PEEK));
            assert_eq!(old_tab.left + shift_x, new_tab.left);
            assert_eq!(old_tab.top + shift_y, new_tab.top);
            assert_eq!(old_tab.right + shift_x, new_tab.right);
            assert_eq!(old_tab.bottom + shift_y, new_tab.bottom);
            let old_x = 250;
            let old_y = 250;
            let new_x = old_x - shift_x;
            let new_y = old_y - shift_y;
            assert_eq!(old_x + old.left, new_x + new.left);
            assert_eq!(old_y + old.top, new_y + new.top);
            assert_eq!(old_x + old_tab.left, new_x + new_tab.left);
            assert_eq!(old_y + old_tab.top, new_y + new_tab.top);

            let (flush_x, flush_y) = match edge {
                FloatingEdge::Left => (monitor.left - old.left, 250),
                FloatingEdge::Right => (monitor.right - old.right, 250),
                FloatingEdge::Top => (250, monitor.top - old.top),
                FloatingEdge::Bottom => (250, monitor.bottom - old.bottom),
            };
            assert!(!floating_drag_has_cleared_edge(
                flush_x, flush_y, old, monitor, Some(edge),
            ), "glow must stay clipped while the panel is flush");
            let (moved_x, moved_y) = match edge {
                FloatingEdge::Left => (flush_x + sc(2), flush_y),
                FloatingEdge::Right => (flush_x - sc(2), flush_y),
                FloatingEdge::Top => (flush_x, flush_y + sc(2)),
                FloatingEdge::Bottom => (flush_x, flush_y - sc(2)),
            };
            assert!(floating_drag_has_cleared_edge(
                moved_x, moved_y, old, monitor, Some(edge),
            ), "external glow must unlock during the drag, not on release");

            let style = ThemeStyle::preset(true, ThemePreset::Forest);
            let accent = drag_hover_outline_color(&style);
            let mut pixels = vec![0u32; (nw * nh) as usize];
            draw_drag_hover_outline(
                &mut pixels, nw, nh, new, style.panel_corner_radius, accent,
            );
            let mx = (new.left + new.right) / 2;
            let my = (new.top + new.bottom) / 2;
            assert_eq!(pixels[(my * nw + mx) as usize], 0,
                "glow must not color the actual panel interior");
            let outside = match edge {
                FloatingEdge::Left => (new.left - 2, my),
                FloatingEdge::Right => (new.right + 2, my),
                FloatingEdge::Top => (mx, new.top - 2),
                FloatingEdge::Bottom => (mx, new.bottom + 2),
            };
            assert!((outside.0 >= 0) && (outside.0 < nw)
                && (outside.1 >= 0) && (outside.1 < nh));
            assert!(pixels[(outside.1 * nw + outside.0) as usize] >> 24 > 0,
                "newly unlocked side must have real external glow");
        }
    }
}

#[test]
fn desktop_taskbar_drag_restores_clipped_glow_in_both_directions() {
    // V24.14: on the bottom taskbar an outward (downward) drag must
    // unlock its missing lower glow, just like an inward (upward) drag.
    // Preserve the old inward-only physical-screen-edge contract.
    let bounds = RECT { left: 100, top: 100, right: 1100, bottom: 850 };
    let threshold = sc(2);
    for (bw, bh) in [(240, 42), (48, 168)] {
        for edge in [
            FloatingEdge::Left, FloatingEdge::Right,
            FloatingEdge::Top, FloatingEdge::Bottom,
        ] {
            let (w, h, body) =
                floating_drag_canvas_geometry(bw, bh, Some(edge), false);
            let (flush_x, flush_y) = match edge {
                FloatingEdge::Left => (bounds.left - body.left, 300),
                FloatingEdge::Right => (bounds.right - body.right, 300),
                FloatingEdge::Top => (300, bounds.top - body.top),
                FloatingEdge::Bottom => (300, bounds.bottom - body.bottom),
            };
            assert!(!floating_drag_has_cleared_edge(
                flush_x, flush_y, body, bounds, Some(edge),
            ));
            assert!(!floating_drag_has_crossed_taskbar_edge(
                flush_x, flush_y, body, bounds, Some(edge),
            ));
            // Outward movement into the taskbar, including the lower
            // side shown in the user's screenshot.
            let (out_x, out_y) = match edge {
                FloatingEdge::Left => (flush_x - threshold, flush_y),
                FloatingEdge::Right => (flush_x + threshold, flush_y),
                FloatingEdge::Top => (flush_x, flush_y - threshold),
                FloatingEdge::Bottom => (flush_x, flush_y + threshold),
            };
            assert!(floating_drag_has_crossed_taskbar_edge(
                out_x, out_y, body, bounds, Some(edge),
            ), "taskbar-side movement must unlock the clipped glow");
            assert!(!floating_drag_has_cleared_edge(
                out_x, out_y, body, bounds, Some(edge),
            ), "old one-sided check must still identify only inward motion");
            let (in_x, in_y) = match edge {
                FloatingEdge::Left => (flush_x + threshold, flush_y),
                FloatingEdge::Right => (flush_x - threshold, flush_y),
                FloatingEdge::Top => (flush_x, flush_y + threshold),
                FloatingEdge::Bottom => (flush_x, flush_y - threshold),
            };
            assert!(floating_drag_has_cleared_edge(
                in_x, in_y, body, bounds, Some(edge),
            ));
            assert!(!floating_drag_has_crossed_taskbar_edge(
                in_x, in_y, body, bounds, Some(edge),
            ));
            // The restored canvas really does contain a drawable gutter
            // beyond the previously taskbar-facing panel edge.
            let (nw, nh, full_body) =
                floating_drag_canvas_geometry(bw, bh, Some(edge), true);
            let accent = drag_hover_outline_color(
                &ThemeStyle::preset(true, ThemePreset::Forest),
            );
            let mut pixels = vec![0u32; (nw * nh) as usize];
            draw_drag_hover_outline(
                &mut pixels, nw, nh, full_body, 8, accent,
            );
            let (px, py) = match edge {
                FloatingEdge::Left => (full_body.left - 2, (full_body.top + full_body.bottom)/2),
                FloatingEdge::Right => (full_body.right + 2, (full_body.top + full_body.bottom)/2),
                FloatingEdge::Top => ((full_body.left + full_body.right)/2, full_body.top - 2),
                FloatingEdge::Bottom => ((full_body.left + full_body.right)/2, full_body.bottom + 2),
            };
            assert!(px >= 0 && px < nw && py >= 0 && py < nh);
            assert!(pixels[(py*nw + px) as usize] >> 24 > 0,
                "full glow must be drawable on formerly clipped side");
            assert!(w <= nw && h <= nh);
        }
    }
}

#[test]
fn desktop_taskbar_drag_does_not_unlock_from_small_jitter() {
    let bounds = RECT { left: 100, top: 100, right: 1100, bottom: 850 };
    let (_, _, body) = floating_canvas_geometry(240, 42, Some(FloatingEdge::Bottom));
    let flush_y = bounds.bottom - body.bottom;
    let t = sc(2);
    for delta in (-t + 1)..t {
        assert!(!floating_drag_has_cleared_edge(
            300, flush_y + delta, body, bounds, Some(FloatingEdge::Bottom),
        ));
        assert!(!floating_drag_has_crossed_taskbar_edge(
            300, flush_y + delta, body, bounds, Some(FloatingEdge::Bottom),
        ));
    }
    assert!(!floating_drag_has_crossed_taskbar_edge(
        300, flush_y + t, body, bounds, None,
    ));
}

#[test]
fn v24_19_transparent_round_corners_do_not_erase_underlying_halo() {
    // The V24.18 source bitmap was copied over a prepainted glow, so
    // alpha-zero corner pixels erased the halo and left a visible gap.
    // At any exterior pixel with glow, source-over must preserve it.
    let mut style = ThemeStyle::preset(true, ThemePreset::Ocean);
    for radius in [0_u8, 3, 8, 16, 22] {
        style.panel_corner_radius = radius;
        for edge in [None, Some(FloatingEdge::Left),
            Some(FloatingEdge::Right), Some(FloatingEdge::Top),
            Some(FloatingEdge::Bottom)] {
            let (w, h, body) = floating_drag_canvas_geometry(
                240, 60, edge, edge.is_some(),
            );
            let tab = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
            let mut halo = vec![0_u32; (w * h) as usize];
            draw_drag_hover_outer_glow(
                &mut halo, w, h, body, tab, &style,
                drag_hover_outline_color(&style),
            );
            let mut corner_pixels = 0;
            for rect in [body, tab] {
                let rw = rect.right - rect.left;
                let rh = rect.bottom - rect.top;
                for y in rect.top.max(0)..rect.bottom.min(h) {
                    for x in rect.left.max(0)..rect.right.min(w) {
                        if panel_outer_shape_contains(
                            rw, rh, x - rect.left, y - rect.top, radius,
                        ) { continue; }
                        let index = (y * w + x) as usize;
                        if halo[index] >> 24 == 0 { continue; }
                        assert_eq!(composite_premultiplied_pixel(halo[index], 0),
                            halo[index],
                            "transparent rounded corner must not punch a dark gap");
                        corner_pixels += 1;
                    }
                }
            }
            assert!(corner_pixels > 0,
                "halo must pass through transparent pixels beside the rounded contour");
        }
    }
}

#[test]
fn v24_19_border_underside_continuity_without_inner_tint() {
    for radius in [0_u8, 2, 7, 15, 22] {
        for alpha in [24_u8, 96, 180, 255] {
            let mut style = ThemeStyle::preset(true, ThemePreset::Forest);
            style.panel_corner_radius = radius;
            style.set_color(StyleColorTarget::PanelBorder,
                Color::rgba(68, 108, 136, alpha));
            style.set_color(StyleColorTarget::PanelBackground,
                Color::rgba(20, 35, 52, alpha));
            for edge in [None, Some(FloatingEdge::Left),
                Some(FloatingEdge::Right), Some(FloatingEdge::Top),
                Some(FloatingEdge::Bottom)] {
                let (w, h, body) = floating_drag_canvas_geometry(
                    240, 64, edge, edge.is_some(),
                );
                let tab = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
                let mut halo = vec![0_u32; (w*h) as usize];
                draw_drag_hover_outer_glow(
                    &mut halo, w, h, body, tab, &style,
                    drag_hover_outline_color(&style),
                );
                let mut border_count = 0;
                for rect in [body, tab] {
                    let rw = rect.right - rect.left;
                    let rh = rect.bottom - rect.top;
                    for y in rect.top.max(0)..rect.bottom.min(h) {
                        for x in rect.left.max(0)..rect.right.min(w) {
                            let lx = x - rect.left;
                            let ly = y - rect.top;
                            let index = (y*w + x) as usize;
                            let is_border = panel_outer_shape_contains(
                                rw, rh, lx, ly, radius,
                            ) && !panel_inner_shape_contains(
                                rw, rh, lx, ly, radius,
                            );
                            let is_content = panel_inner_shape_contains(
                                rw, rh, lx, ly, radius,
                            );
                            if is_content {
                                assert_eq!(halo[index], 0,
                                    "no underpaint under the translucent panel content");
                            } else if is_border && halo[index] >> 24 > 0 {
                                border_count += 1;
                                let border = premultiplied_pixel(
                                    style.color(StyleColorTarget::PanelBorder),
                                );
                                let result = composite_premultiplied_pixel(
                                    halo[index], border,
                                );
                                if alpha == 255 {
                                    assert_eq!(result, border,
                                        "opaque border completely obscures underpaint");
                                } else {
                                    assert_ne!(result, border,
                                        "semi-transparent border keeps a narrow seamless glow");
                                }
                            }
                        }
                    }
                }
                assert!(border_count > 0,
                    "underpaint must contact the actual outer border");
            }
        }
    }
}

#[test]
fn v24_19_premultiplied_foreground_source_over_preserves_color_rules() {
    let under = premultiplied_pixel(Color::rgba(20, 180, 240, 175));
    assert_eq!(composite_premultiplied_pixel(under, 0), under);
    assert_eq!(composite_premultiplied_pixel(0, under), under);
    let solid = premultiplied_pixel(Color::rgba(30, 50, 70, 255));
    assert_eq!(composite_premultiplied_pixel(under, solid), solid);
    let translucent = premultiplied_pixel(Color::rgba(100, 120, 160, 85));
    let mixed = composite_premultiplied_pixel(under, translucent);
    assert!(mixed >> 24 > under >> 24);
    for shift in [0, 8, 16] {
        assert!((mixed >> shift) & 255 <= mixed >> 24,
            "result must remain premultiplied for UpdateLayeredWindow");
    }
}

#[test]
fn v24_18_outer_glow_leaves_all_panel_and_tab_pixels_untinted() {
    // Desktop overlay and glass surfaces may be translucent: painting
    // the halo behind the foreground is not sufficient without a strict
    // union silhouette mask, since inner light would show through.
    for radius in [0_u8, 2, 8, 20] {
        for alpha in [32_u8, 105, 255] {
            let mut style = ThemeStyle::preset(false, ThemePreset::Ocean);
            style.panel_corner_radius = radius;
            style.set_color(StyleColorTarget::PanelBackground,
                Color::rgba(23, 42, 68, alpha));
            style.set_color(StyleColorTarget::PanelBorder,
                Color::rgba(67, 123, 151, alpha));
            let accent = drag_hover_outline_color(&style);
            for edge in [None, Some(FloatingEdge::Left),
                Some(FloatingEdge::Right), Some(FloatingEdge::Top),
                Some(FloatingEdge::Bottom)] {
                let (w, h, body) = floating_drag_canvas_geometry(
                    240, 52, edge, edge.is_some(),
                );
                let tab = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
                let mut foreground = vec![0u32; (w * h) as usize];
                for y in body.top..body.bottom {
                    for x in body.left..body.right {
                        if let Some(color) = panel_color_at(
                            &style, body.right - body.left, body.bottom - body.top,
                            x - body.left, y - body.top,
                        ) {
                            foreground[(y * w + x) as usize] = premultiplied_pixel(color);
                        }
                    }
                }
                render_floating_handle_bitmap(
                    &mut foreground, w, h, edge, true, &style, 100,
                );
                let mut composite = vec![0u32; (w * h) as usize];
                draw_drag_hover_outer_glow(
                    &mut composite, w, h, body, tab, &style, accent,
                );
                let mut halo_found = false;
                for y in 0..h {
                    for x in 0..w {
                        let index = (y * w + x) as usize;
                        let body_mask = panel_outer_shape_contains(
                            body.right - body.left, body.bottom - body.top,
                            x - body.left, y - body.top, radius,
                        );
                        let tab_mask = panel_outer_shape_contains(
                            tab.right - tab.left, tab.bottom - tab.top,
                            x - tab.left, y - tab.top, radius,
                        );
                        let in_content = panel_inner_shape_contains(
                            body.right - body.left, body.bottom - body.top,
                            x - body.left, y - body.top, radius,
                        ) || panel_inner_shape_contains(
                            tab.right - tab.left, tab.bottom - tab.top,
                            x - tab.left, y - tab.top, radius,
                        );
                        if in_content {
                            assert_eq!(composite[index], 0,
                                "halo must never underpaint the inner fill area");
                        } else if !(body_mask || tab_mask)
                            && composite[index] >> 24 > 0 {
                            halo_found = true;
                        }
                        // Production uses source-over, not direct copy.
                        // Inner fill has no light behind and must remain
                        // bit-for-bit identical across translucent themes.
                        let blended = composite_premultiplied_pixel(
                            composite[index], foreground[index],
                        );
                        if in_content {
                            assert_eq!(blended, foreground[index],
                                "inner text and panel colors must remain untinted");
                        }
                        composite[index] = blended;
                    }
                }
                assert!(halo_found, "outer halo must remain visible at every dock edge");
            }
        }
    }
}

#[test]
fn v24_18_outer_glow_contacts_actual_rounded_contour_without_any_gap() {
    // At every four-neighbor boundary of the true panel or drag tab,
    // the directly adjacent outside pixel must receive light. This
    // guards the circular corner seam that made an earlier outside-only
    // experiment look detached from the panel.
    let mut style = ThemeStyle::preset(true, ThemePreset::Forest);
    for radius in [0_u8, 2, 5, 12, 20] {
        style.panel_corner_radius = radius;
        for edge in [None, Some(FloatingEdge::Left),
            Some(FloatingEdge::Right), Some(FloatingEdge::Top),
            Some(FloatingEdge::Bottom)] {
            let (w, h, body) = floating_drag_canvas_geometry(
                230, 70, edge, edge.is_some(),
            );
            let tab = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
            let mask_at = |x: i32, y: i32| {
                [body, tab].iter().any(|rect| panel_outer_shape_contains(
                    rect.right - rect.left, rect.bottom - rect.top,
                    x - rect.left, y - rect.top, radius,
                ))
            };
            let mut pixels = vec![0u32; (w * h) as usize];
            draw_drag_hover_outer_glow(
                &mut pixels, w, h, body, tab,
                &style, drag_hover_outline_color(&style),
            );
            let mut boundary_pairs = 0;
            for y in 0..h {
                for x in 0..w {
                    if !mask_at(x, y) { continue; }
                    for (dx, dy) in [(1,0), (-1,0), (0,1), (0,-1)] {
                        let px = x + dx;
                        let py = y + dy;
                        if px < 0 || px >= w || py < 0 || py >= h
                            || mask_at(px, py) { continue; }
                        let opacity = pixels[(py*w + px) as usize] >> 24;
                        assert!(opacity > 0,
                            "glow must touch every rounded exterior contour pixel, no gap");
                        if [body, tab].iter().any(|rect| panel_inner_shape_contains(
                            rect.right - rect.left, rect.bottom - rect.top,
                            x - rect.left, y - rect.top, radius,
                        )) {
                            assert_eq!(pixels[(y*w + x) as usize], 0,
                                "halo cannot reach the painted content region");
                        } else {
                            assert!(pixels[(y*w + x) as usize] >> 24 > 0,
                                "the narrow outer border needs a subpixel underpaint");
                        }
                        boundary_pairs += 1;
                    }
                }
            }
            assert!(boundary_pairs > 10);
        }
    }
}

#[test]
fn v24_18_nearby_panel_and_tab_glows_do_not_tint_each_other() {
    let mut style = ThemeStyle::preset(true, ThemePreset::Ocean);
    style.panel_corner_radius = 10;
    for edge in [None, Some(FloatingEdge::Left),
        Some(FloatingEdge::Right), Some(FloatingEdge::Top),
        Some(FloatingEdge::Bottom)] {
        let (w, h, body) = floating_canvas_geometry(240, 52, edge);
        let tab = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
        let mut pixels = vec![0u32; (w*h) as usize];
        draw_drag_hover_outer_glow(
            &mut pixels, w, h, body, tab, &style,
            drag_hover_outline_color(&style),
        );
        for y in 0..h {
            for x in 0..w {
                if [body, tab].iter().any(|r| panel_inner_shape_contains(
                    r.right-r.left, r.bottom-r.top,
                    x-r.left, y-r.top, style.panel_corner_radius,
                )) {
                    assert_eq!(pixels[(y*w+x) as usize], 0);
                }
            }
        }
    }
}

#[test]
fn v24_4_docked_glow_gutters_respect_physical_monitor_edges() {
    let g = sc(FLOAT_FREE_GLOW_MARGIN);
    let peek = sc(FLOAT_EDGE_PEEK);
    let bounds = RECT { left: -1920, top: -80, right: 0, bottom: 1000 };
    let style = ThemeStyle::preset(true, ThemePreset::Forest);
    for (bw, bh) in [(240, 42), (48, 168)] {
        for edge in [FloatingEdge::Left, FloatingEdge::Right,
            FloatingEdge::Top, FloatingEdge::Bottom] {
            let (cw, ch, body) = floating_canvas_geometry(bw, bh, Some(edge));
            let grip = floating_grip_rect(cw, ch, Some(edge), peek);
            assert_eq!(body.right - body.left, bw);
            assert_eq!(body.bottom - body.top, bh);
            // Outer gutter on the three sides facing the inside of the
            // monitor; no fake blank strip against the physical edge.
            match edge {
                FloatingEdge::Left => {
                    assert_eq!(body.left, 0);
                    assert_eq!(body.top, g);
                    assert_eq!(ch - body.bottom, g);
                    assert_eq!(cw - grip.right, g);
                    assert_eq!(grip.left, body.right);
                }
                FloatingEdge::Right => {
                    assert_eq!(cw - body.right, 0);
                    assert_eq!(body.top, g);
                    assert_eq!(ch - body.bottom, g);
                    assert_eq!(grip.left, g);
                    assert_eq!(grip.right, body.left);
                }
                FloatingEdge::Top => {
                    assert_eq!(body.top, 0);
                    assert_eq!(body.left, g);
                    assert_eq!(cw - body.right, g);
                    assert_eq!(ch - grip.bottom, g);
                    assert_eq!(grip.top, body.bottom);
                }
                FloatingEdge::Bottom => {
                    assert_eq!(ch - body.bottom, 0);
                    assert_eq!(body.left, g);
                    assert_eq!(cw - body.right, g);
                    assert_eq!(grip.top, g);
                    assert_eq!(grip.bottom, body.top);
                }
            }
            let expanded = floating_edge_position(
                bounds, bounds.left + 300, bounds.top + 300, cw, ch,
                Some(edge), false, peek,
            );
            let collapsed = floating_edge_position(
                bounds, bounds.left + 300, bounds.top + 300, cw, ch,
                Some(edge), true, peek,
            );
            let grip_screen = RECT {
                left: collapsed.0 + grip.left, right: collapsed.0 + grip.right,
                top: collapsed.1 + grip.top, bottom: collapsed.1 + grip.bottom,
            };
            match edge {
                FloatingEdge::Left => {
                    assert_eq!(expanded.0 + body.left, bounds.left);
                    assert_eq!(grip_screen.left, bounds.left);
                }
                FloatingEdge::Right => {
                    assert_eq!(expanded.0 + body.right, bounds.right);
                    assert_eq!(grip_screen.right, bounds.right);
                }
                FloatingEdge::Top => {
                    assert_eq!(expanded.1 + body.top, bounds.top);
                    assert_eq!(grip_screen.top, bounds.top);
                }
                FloatingEdge::Bottom => {
                    assert_eq!(expanded.1 + body.bottom, bounds.bottom);
                    assert_eq!(grip_screen.bottom, bounds.bottom);
                }
            }
            assert!(floating_drag_handle_hit(
                (grip.left + grip.right) / 2, (grip.top + grip.bottom) / 2,
                cw, ch, Some(edge), peek,
            ));
            let mut pixels = vec![0u32; (cw * ch) as usize];
            draw_drag_hover_outline(
                &mut pixels, cw, ch, body,
                style.panel_corner_radius, drag_hover_outline_color(&style),
            );
            let center_y = (body.top + body.bottom) / 2;
            let center_x = (body.left + body.right) / 2;
            let al = |x: i32, y: i32| -> u32 {
                pixels[(y * cw + x) as usize] >> 24
            };
            match edge {
                FloatingEdge::Left => {
                    assert!(al(body.right + 2, center_y) > 0);
                    assert!(al(center_x, body.top - 2) > 0);
                    assert!(al(center_x, body.bottom + 2) > 0);
                }
                FloatingEdge::Right => {
                    assert!(al(body.left - 2, center_y) > 0);
                    assert!(al(center_x, body.top - 2) > 0);
                    assert!(al(center_x, body.bottom + 2) > 0);
                }
                FloatingEdge::Top => {
                    assert!(al(body.left - 2, center_y) > 0);
                    assert!(al(body.right + 2, center_y) > 0);
                    assert!(al(center_x, body.bottom + 2) > 0);
                }
                FloatingEdge::Bottom => {
                    assert!(al(body.left - 2, center_y) > 0);
                    assert!(al(body.right + 2, center_y) > 0);
                    assert!(al(center_x, body.top - 2) > 0);
                }
            }
            // Glow stays visual-only: the component center remains clear.
            assert_eq!(al(center_x, center_y), 0);
        }
    }
}

#[test]
fn desktop_repeated_direction_switch_preserves_panel_and_external_grip() {
    // Reproduces the user's rapid horizontal/vertical toggles at the pure
    // geometry layer. Every switch must preserve the panel and its
    // external drag area without accumulating size or offset errors.
    let horizontal = (240, 42);
    let vertical = (48, 184);
    let edges = [
        None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
        Some(FloatingEdge::Top), Some(FloatingEdge::Bottom),
    ];
    for i in 0..1_000 {
        let (body_w, body_h) = if i % 2 == 0 { horizontal } else { vertical };
        for edge in edges {
            let (canvas_w, canvas_h, body) =
                floating_canvas_geometry(body_w, body_h, edge);
            let grip = floating_grip_rect(
                canvas_w, canvas_h, edge, sc(FLOAT_EDGE_PEEK),
            );
            assert_eq!(body.right - body.left, body_w);
            assert_eq!(body.bottom - body.top, body_h);
            assert!(grip.left >= 0 && grip.top >= 0
                && grip.right <= canvas_w && grip.bottom <= canvas_h);
            assert!(grip.right <= body.left || grip.left >= body.right
                || grip.bottom <= body.top || grip.top >= body.bottom);
        }
    }
}

#[test]
fn desktop_grip_stays_inside_only_visible_edge_peek() {
    let width = 240;
    let height = 168;
    let peek = sc(FLOAT_EDGE_PEEK);
    for edge in [Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                 Some(FloatingEdge::Top), Some(FloatingEdge::Bottom), None] {
        let r = floating_grip_rect(width, height, edge, peek);
        assert!(r.left >= 0 && r.top >= 0 && r.right <= width && r.bottom <= height);
        assert!(floating_drag_handle_hit((r.left + r.right) / 2,
            (r.top + r.bottom) / 2, width, height, edge, peek));
        assert!(!floating_drag_handle_hit(-1, -1, width, height, edge, peek));
        match edge {
            Some(FloatingEdge::Left) => {
                assert_eq!(r.left, width - sc(FLOAT_FREE_GLOW_MARGIN) - peek);
            }
            Some(FloatingEdge::Right) => {
                assert_eq!(r.right, sc(FLOAT_FREE_GLOW_MARGIN) + peek);
            }
            Some(FloatingEdge::Top) => {
                assert_eq!(r.bottom, height - sc(FLOAT_FREE_GLOW_MARGIN));
            }
            Some(FloatingEdge::Bottom) => {
                assert_eq!(r.top, sc(FLOAT_FREE_GLOW_MARGIN));
            },
            None => assert_eq!(r.top, sc(FLOAT_FREE_GLOW_MARGIN)),
        }
    }
}

#[test]
fn desktop_drag_grip_geometry_stays_fixed_until_release() {
    let body_w = 240;
    let body_h = 168;
    for edge in [FloatingEdge::Left, FloatingEdge::Right,
                 FloatingEdge::Top, FloatingEdge::Bottom] {
        let (w, h, body) = floating_canvas_geometry(body_w, body_h, Some(edge));
        let original_tab = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
        let anchor_x = (original_tab.left + original_tab.right) / 2;
        let anchor_y = (original_tab.top + original_tab.bottom) / 2;
        for pointer in [POINT { x: 300, y: 400 }, POINT { x: 700, y: 250 }] {
            let window_x = pointer.x - anchor_x;
            let window_y = pointer.y - anchor_y;
            let tab = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
            assert_eq!((window_x + tab.left + (tab.right - tab.left) / 2,
                        window_y + tab.top + (tab.bottom - tab.top) / 2),
                        (pointer.x, pointer.y));
            assert_eq!((body.right - body.left, body.bottom - body.top),
                       (body_w, body_h));
        }
    }
}

#[test]
fn desktop_grip_fade_is_reversible_and_never_changes_body_pixels() {
    assert_eq!(eased_value_at(0, 100, Duration::ZERO, 140), 0);
    assert_eq!(eased_value_at(0, 100, Duration::from_millis(70), 140), 88);
    assert_eq!(eased_value_at(0, 100, Duration::from_millis(140), 140), 100);
    assert_eq!(eased_value_at(100, 0, Duration::ZERO, 170), 100);
    assert_eq!(eased_value_at(100, 0, Duration::from_millis(85), 170), 50);
    assert_eq!(eased_value_at(100, 0, Duration::from_millis(170), 170), 0);
    let width = 240;
    let height = 168;
    let style = ThemeStyle::light_default();
    let grip = floating_grip_rect(width, height, None, sc(FLOAT_EDGE_PEEK));
    let mut body = vec![0xFF445566u32; (width * height) as usize];
    for y in grip.top..grip.bottom {
        for x in grip.left..grip.right {
            body[(y * width + x) as usize] = 0; // detached tab gutter
        }
    }
    let mut hidden = body.clone();
    let mut visible = body.clone();
    render_floating_handle_bitmap(
        &mut hidden, width, height, None, false, &style, 0,
    );
    render_floating_handle_bitmap(
        &mut visible, width, height, None, false, &style, 100,
    );
    for y in 0..height {
        for x in 0..width {
            let idx = (y * width + x) as usize;
            if !point_in_rect(grip, x, y) {
                assert_eq!(hidden[idx], body[idx]);
                assert_eq!(visible[idx], body[idx]);
            } else {
                assert_eq!((hidden[idx] >> 24) & 0xFF, 0,
                    "fully transparent free grip must not intercept clicks");
            }
        }
    }
}

#[test]
fn v24_drag_grip_dots_remain_stable_on_hover() {
    let base = Color::rgba(40, 70, 110, 160);
    for vertical in [false, true] {
        assert_eq!(
            drag_handle_dot_style(base, false, vertical),
            drag_handle_dot_style(base, true, vertical),
            "hover must no longer grow the grip under the mouse",
        );
    }
}

#[test]
fn v24_hover_color_follows_theme_and_stays_readable() {
    for dark in [false, true] {
        for preset in ThemePreset::ALL {
            let style = ThemeStyle::preset(dark, preset);
            let color = drag_hover_outline_color(&style);
            assert_eq!(color.a, 255);
            let background = style.color(StyleColorTarget::PanelBackground);
            let difference = (i32::from(color.r) - i32::from(background.r)).abs()
                + (i32::from(color.g) - i32::from(background.g)).abs()
                + (i32::from(color.b) - i32::from(background.b)).abs();
            assert!(difference >= 150, "theme hover outline must remain visible");
            assert_eq!(color, Color::rgba(
                style.color(StyleColorTarget::ProgressHigh).r,
                style.color(StyleColorTarget::ProgressHigh).g,
                style.color(StyleColorTarget::ProgressHigh).b,
                255,
            ));
        }
    }
    let mut custom = ThemeStyle::dark_default();
    custom.set_color(StyleColorTarget::ProgressHigh,
        custom.color(StyleColorTarget::PanelBackground));
    assert_ne!(drag_hover_outline_color(&custom),
        custom.color(StyleColorTarget::ProgressHigh));
}

#[test]
fn v24_1_glow_is_feathered_not_a_thick_solid_outline() {
    assert!(drag_hover_glow_coverage(0.0) > drag_hover_glow_coverage(1.0));
    assert!(drag_hover_glow_coverage(1.0) > drag_hover_glow_coverage(2.0));
    assert!(drag_hover_glow_coverage(2.0) > drag_hover_glow_coverage(3.0));
    assert!(drag_hover_glow_coverage(3.0) > drag_hover_glow_coverage(4.0));
    assert_eq!(drag_hover_glow_coverage(0.0), drag_hover_glow_coverage(-0.0));
    assert_eq!(drag_hover_glow_coverage(2.0), drag_hover_glow_coverage(-2.0));
    let style = ThemeStyle::preset(true, ThemePreset::Forest);
    let mut pixels = vec![0u32; 190 * 62];
    let body = RECT { left: 7, top: 7, right: 183, bottom: 55 };
    draw_drag_hover_outline(&mut pixels, 190, 62, body,
        style.panel_corner_radius, drag_hover_outline_color(&style));
    let sample_alpha = |y: usize| -> u32 { pixels[y * 190 + 95] >> 24 };
    // There is real translucent glow outside a panel when canvas space
    // exists, but it never paints the center or expands the body geometry.
    assert!(sample_alpha(6) > 0, "halo should extend beyond outer body edge");
    assert!(sample_alpha(6) < sample_alpha(8));
    assert!(sample_alpha(8) > sample_alpha(10));
    assert!(sample_alpha(10) > sample_alpha(12));
    assert_eq!(sample_alpha(20), 0);
    assert_eq!(pixels.len(), 190 * 62);
}

#[test]
fn v24_hover_outlines_only_panel_and_detached_tab_without_resizing() {
    let style = ThemeStyle::preset(true, ThemePreset::Forest);
    let accent = drag_hover_outline_color(&style);
    for (w, h) in [(180, 40), (50, 168)] {
        let bounds = RECT { left: 0, top: 0, right: w, bottom: h };
        let mut pixels = vec![0xFF14211Du32; (w * h) as usize];
        let idle = pixels.clone();
        draw_drag_hover_outline(&mut pixels, w, h, bounds,
            style.panel_corner_radius, accent);
        assert_ne!(pixels, idle);
        assert_eq!(pixels[(h / 2 * w + w / 2) as usize],
            idle[(h / 2 * w + w / 2) as usize],
            "center content must never be covered by the hover ring");
        assert_ne!(pixels[(w + w / 2) as usize],
            idle[(w + w / 2) as usize], "top border must highlight");
    }
    for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
        Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
        let (cw, ch, body) = floating_canvas_geometry(180, 40, edge);
        let tab = floating_grip_rect(cw, ch, edge, sc(FLOAT_EDGE_PEEK));
        let mut canvas = vec![0u32; (cw * ch) as usize];
        draw_drag_hover_outline(&mut canvas, cw, ch, body,
            style.panel_corner_radius, accent);
        let after_body = canvas.clone();
        draw_drag_hover_outline(&mut canvas, cw, ch, tab,
            style.panel_corner_radius, accent);
        assert_ne!(canvas, after_body, "desktop tab must highlight independently");
        assert_eq!(canvas.len(), (cw * ch) as usize);
        let body_center = ((body.top + body.bottom) / 2 * cw
            + (body.left + body.right) / 2) as usize;
        assert_eq!(canvas[body_center], 0,
            "independent hover borders must not fill the actual content");
    }
}

#[test]
fn desktop_transparent_drag_tab_whole_rounded_surface_remains_hit_testable() {
    // UpdateLayeredWindow discards alpha-zero pixels before WM_NCHITTEST.
    // An invisible 1/255 alpha probe in the *real* grip silhouette must
    // make empty background cells draggable, without showing a fill.
    for (body_w, body_h) in [(240, 42), (48, 168)] {
        for edge in [None, Some(FloatingEdge::Left),
            Some(FloatingEdge::Right), Some(FloatingEdge::Top),
            Some(FloatingEdge::Bottom)] {
            let (w, h, _) = floating_canvas_geometry(body_w, body_h, edge);
            let grip = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
            for radius in [0_u8, 3, 10, 20] {
                let mut style = ThemeStyle::preset(true, ThemePreset::Ocean);
                style.panel_corner_radius = radius;
                style.set_color(StyleColorTarget::PanelBackground,
                    Color::rgba(25, 42, 66, 0));
                style.set_color(StyleColorTarget::PanelBorder,
                    Color::rgba(61, 103, 143, 255));
                for hovered in [false, true] {
                    let mut pixels = vec![0u32; (w * h) as usize];
                    render_floating_handle_bitmap(
                        &mut pixels, w, h, edge, hovered, &style, 100,
                    );
                    let mut empty_cells = 0;
                    for y in 0..h {
                        for x in 0..w {
                            let index = (y * w + x) as usize;
                            let inside = panel_outer_shape_contains(
                                grip.right - grip.left, grip.bottom - grip.top,
                                x - grip.left, y - grip.top, radius,
                            );
                            if inside {
                                assert!(pixels[index] >> 24 > 0,
                                    "even a fully transparent grip interior must receive mouse events");
                                assert_eq!(floating_drag_tab_hit_code(
                                    RECT::default(), grip, x, y,
                                ), 1);
                                assert!(floating_drag_handle_hit(
                                    x, y, w, h, edge, sc(FLOAT_EDGE_PEEK),
                                ));
                                if panel_inner_shape_contains(
                                    grip.right - grip.left, grip.bottom - grip.top,
                                    x - grip.left, y - grip.top, radius,
                                ) && pixels[index] == 0x01000000 {
                                    empty_cells += 1;
                                }
                            } else {
                                assert_eq!(pixels[index], 0,
                                    "rounded transparent corners and external glow gutter must remain click-through");
                            }
                        }
                    }
                    assert!(empty_cells > 0,
                        "a transparent drag tab must contain invisible clickable blank pixels");
                }
            }
        }
    }
}

#[test]
fn desktop_transparent_drag_tab_probe_respects_fade_out_and_rounded_corners() {
    let (w, h, _) = floating_canvas_geometry(240, 42, None);
    let grip = floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK));
    let mut style = ThemeStyle::preset(true, ThemePreset::Ocean);
    style.panel_corner_radius = 10;
    style.set_color(StyleColorTarget::PanelBackground, Color::rgba(0, 0, 0, 0));
    style.set_color(StyleColorTarget::PanelBorder, Color::rgba(50, 80, 120, 0));
    for opacity in [1_u8, 25, 100] {
        let mut pixels = vec![0u32; (w * h) as usize];
        render_floating_handle_bitmap(&mut pixels, w, h, None, false, &style, opacity);
        let x = grip.left + sc(6);
        let y = (grip.top + grip.bottom) / 2;
        let pixel = pixels[(y * w + x) as usize];
        assert!(pixel >> 24 > 0);
        assert!(pixels.contains(&0x01000000),
            "transparent tab retains an imperceptible premultiplied mouse-hit surface");
        assert_eq!(pixels[0], 0,
            "outside the actual drag block remains transparent");
    }
    let mut faded_out = vec![0u32; (w * h) as usize];
    render_floating_handle_bitmap(&mut faded_out, w, h, None, false, &style, 0);
    assert!(faded_out.iter().all(|pixel| *pixel == 0),
        "a genuinely faded-out free tab must remain noninteractive");
}

#[test]
fn desktop_mini_grip_matches_panel_palette_corner_radius_and_alpha() {
    let mut style = ThemeStyle::dark_default();
    style.set_color(StyleColorTarget::PanelBackground, Color::rgba(14, 32, 25, 68));
    style.set_color(StyleColorTarget::PanelBorder, Color::rgba(40, 110, 87, 127));
    style.panel_corner_radius = 2;
    let (w, h, _) = floating_canvas_geometry(
        sc(FLOAT_GRIP_LENGTH), sc(FLOAT_EDGE_PEEK), None,
    );
    let grip = floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK));
    assert_eq!(grip.right - grip.left, sc(36));
    assert_eq!(grip.bottom - grip.top, sc(16));
    let mut pixels = vec![0u32; (w * h) as usize];
    render_floating_handle_bitmap(&mut pixels, w, h, None, false, &style, 100);
    let center = ((grip.top + grip.bottom) / 2 * w + grip.left + sc(6)) as usize;
    assert_eq!(pixels[center], premultiplied_pixel(
        style.color(StyleColorTarget::PanelBackground),
    ));
    // Panel corner clipping and the new outer halo gutter stay transparent.
    assert_eq!(pixels[0], 0);
    let mut transparent = vec![0u32; (w * h) as usize];
    render_floating_handle_bitmap(&mut transparent, w, h, None, false, &style, 0);
    assert!(transparent.iter().all(|&pixel| pixel == 0));
}

#[test]
fn desktop_edge_slide_easing_is_time_based_and_directional() {
    assert_eq!(eased_value_at(0, 100, Duration::ZERO, 220), 0);
    assert_eq!(eased_value_at(0, 100, Duration::from_millis(110), 220), 88);
    assert_eq!(eased_value_at(100, 0, Duration::from_millis(120), 240), 50);
    assert_eq!(eased_value_at(0, 100, Duration::from_millis(220), 220), 100);
    assert_eq!(eased_value_at(100, 0, Duration::from_millis(240), 240), 0);
    assert_eq!(animation_partial_duration(240, 50, 0), 120);
    assert_eq!(animation_partial_duration(240, 100, 0), 240);
}

#[test]
fn desktop_first_snap_retraction_ignores_hover_until_rearmed() {
    assert!(!floating_hover_can_reveal(true, true, false));
    assert!(!floating_hover_can_reveal(true, false, true));
    assert!(!floating_hover_can_reveal(false, false, false));
    assert!(floating_hover_can_reveal(true, false, false));
    // The first hidden tab remains visible, but the cursor crossing its
    // path during collapse cannot reverse the animation.
    assert_eq!(eased_value_at(
        100, 0, Duration::from_millis(FLOAT_RETRACT_DURATION_MS / 2),
        FLOAT_RETRACT_DURATION_MS,
    ), 50);
    assert_eq!(eased_value_at(
        100, 0, Duration::from_millis(FLOAT_RETRACT_DURATION_MS),
        FLOAT_RETRACT_DURATION_MS,
    ), 0);
}

#[test]
fn desktop_corner_snap_picks_nearest_mouse_edge_at_all_four_corners() {
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    // Both HWND edges cross each physical corner. The old fixed
    // left/right/top/bottom priority was independent of cursor position.
    let scenarios = [
        (RECT { left: -10, top: -10, right: 220, bottom: 120 },
            POINT { x: 4, y: 18 }, FloatingEdge::Left),
        (RECT { left: -10, top: -10, right: 220, bottom: 120 },
            POINT { x: 18, y: 4 }, FloatingEdge::Top),
        (RECT { left: 1800, top: -10, right: 1950, bottom: 120 },
            POINT { x: 1916, y: 20 }, FloatingEdge::Right),
        (RECT { left: 1800, top: -10, right: 1950, bottom: 120 },
            POINT { x: 1895, y: 2 }, FloatingEdge::Top),
        (RECT { left: -10, top: 980, right: 220, bottom: 1110 },
            POINT { x: 3, y: 1056 }, FloatingEdge::Left),
        (RECT { left: -10, top: 980, right: 220, bottom: 1110 },
            POINT { x: 24, y: 1078 }, FloatingEdge::Bottom),
        (RECT { left: 1790, top: 950, right: 1950, bottom: 1110 },
            POINT { x: 1918, y: 1045 }, FloatingEdge::Right),
        (RECT { left: 1790, top: 950, right: 1950, bottom: 1110 },
            POINT { x: 1880, y: 1077 }, FloatingEdge::Bottom),
    ];
    for (widget, cursor, expected) in scenarios {
        assert_eq!(floating_drop_edge_at_pointer(
            monitor, monitor, widget, cursor, 24, [true; 4],
        ), Some(expected), "mouse=({}, {}) should choose nearest corner edge",
            cursor.x, cursor.y);
    }
}

#[test]
fn desktop_taskbar_corner_snap_uses_actual_inner_taskbar_edge_distances() {
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    // Bottom and left taskbars both inset from the physical monitor.
    // The mouse lies inside both reserved bands: compare distances to
    // x=48 and y=1032 instead of taking the old hard-coded Left.
    let work = RECT { left: 48, top: 0, right: 1920, bottom: 1032 };
    let widget = RECT { left: 35, top: 1000, right: 235, bottom: 1060 };
    for (mouse, edge) in [
        (POINT { x: 46, y: 1035 }, FloatingEdge::Left),
        (POINT { x: 45, y: 1033 }, FloatingEdge::Bottom),
        (POINT { x: 20, y: 1050 }, FloatingEdge::Bottom),
    ] {
        let dx = mouse.x.abs_diff(work.left);
        let dy = mouse.y.abs_diff(work.bottom);
        let actual = floating_drop_edge_at_pointer(
            monitor, work, widget, mouse, 24, [true; 4]);
        assert_eq!(actual, Some(edge), "left distance={dx}, bottom distance={dy}");
    }
    // Bottom taskbar meets an unreserved physical screen side.
    let bottom = RECT { left: 0, top: 0, right: 1920, bottom: 1032 };
    let widget = RECT { left: -10, top: 1000, right: 200, bottom: 1070 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, bottom, widget, POINT { x: 3, y: 1052 }, 24, [true; 4],
    ), Some(FloatingEdge::Left));
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, bottom, widget, POINT { x: 30, y: 1038 }, 24, [true; 4],
    ), Some(FloatingEdge::Bottom));
    // Left-side taskbar meets top: choose mouse distance to x=48/y=0.
    let left = RECT { left: 48, top: 0, right: 1920, bottom: 1080 };
    let widget = RECT { left: 20, top: -8, right: 220, bottom: 90 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, left, widget, POINT { x: 60, y: 4 }, 24, [true; 4],
    ), Some(FloatingEdge::Top));
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, left, widget, POINT { x: 49, y: 22 }, 24, [true; 4],
    ), Some(FloatingEdge::Left));
    // Right-side taskbar meets bottom.
    let right = RECT { left: 0, top: 0, right: 1872, bottom: 1080 };
    let widget = RECT { left: 1810, top: 980, right: 1900, bottom: 1110 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, right, widget, POINT { x: 1900, y: 1079 }, 24, [true; 4],
    ), Some(FloatingEdge::Bottom));
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, right, widget, POINT { x: 1874, y: 1045 }, 24, [true; 4],
    ), Some(FloatingEdge::Right));
}

#[test]
fn desktop_shared_seam_corner_uses_nearest_mouse_edge_including_taskbar() {
    // A seam is a normal candidate. The mouse-selected display owns the
    // dock; at its corner the shorter perpendicular distance wins.
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let bottom = RECT { left: 0, top: 0, right: 1920, bottom: 1032 };
    let widget = RECT { left: -10, top: 1010, right: 210, bottom: 1080 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, bottom, widget, POINT { x: 2, y: 1040 }, 24, [true; 4],
    ), Some(FloatingEdge::Left), "left seam is closer than taskbar");
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, bottom, widget, POINT { x: 16, y: 1035 }, 24, [true; 4],
    ), Some(FloatingEdge::Bottom), "taskbar boundary is closer than seam");
    let full = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let widget = RECT { left: -10, top: -10, right: 210, bottom: 100 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, full, widget, POINT { x: 1, y: 3 }, 24, [true; 4],
    ), Some(FloatingEdge::Left));
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, full, widget, POINT { x: 8, y: 1 }, 24, [true; 4],
    ), Some(FloatingEdge::Top));
}

#[test]
fn desktop_taskbar_drop_with_entire_canvas_beyond_work_area_still_snaps() {
    // V24.11 field failure: secondary display taskbar occupies y=912..960.
    // The cursor is inside Explorer; the detached window is completely
    // below the work area, so the old rectangle-only test returns None.
    let monitor = RECT { left: 2048, top: 0, right: 3755, bottom: 960 };
    let work = RECT { left: 2048, top: 0, right: 3755, bottom: 912 };
    let widget = RECT { left: 2500, top: 925, right: 2760, bottom: 1025 };
    let cursor = POINT { x: 2550, y: 954 };
    assert_eq!(floating_drop_edge(work, widget, 24, [true; 4]), None);
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, work, widget, cursor, 24, [true; 4],
    ), Some(FloatingEdge::Bottom));
    let (canvas_w, canvas_h, _) =
        floating_canvas_geometry(240, 48, Some(FloatingEdge::Bottom));
    let (_, snapped_y) = floating_edge_position(
        work, widget.left, widget.top, canvas_w, canvas_h,
        Some(FloatingEdge::Bottom), false, 16,
    );
    assert_eq!(snapped_y + canvas_h, work.bottom);
}

#[test]
fn desktop_taskbar_drop_overshoot_detects_all_four_reserved_edges() {
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let rect = RECT { left: 650, top: 800, right: 880, bottom: 915 };
    for (work, cursor, expected) in [
        (
            RECT { left: 0, top: 0, right: 1920, bottom: 1032 },
            POINT { x: 720, y: 1070 },
            FloatingEdge::Bottom,
        ),
        (
            RECT { left: 0, top: 48, right: 1920, bottom: 1080 },
            POINT { x: 720, y: 5 },
            FloatingEdge::Top,
        ),
        (
            RECT { left: 48, top: 0, right: 1920, bottom: 1080 },
            POINT { x: 4, y: 670 },
            FloatingEdge::Left,
        ),
        (
            RECT { left: 0, top: 0, right: 1872, bottom: 1080 },
            POINT { x: 1915, y: 670 },
            FloatingEdge::Right,
        ),
    ] {
        assert_eq!(floating_drop_edge_at_pointer(
            monitor, work, rect, cursor, 24, [true; 4],
        ), Some(expected));
    }
}

#[test]
fn desktop_taskbar_drop_does_not_snap_when_cursor_not_in_reserved_band() {
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let work = RECT { left: 0, top: 0, right: 1920, bottom: 1032 };
    let off_work = RECT { left: 650, top: 1040, right: 880, bottom: 1135 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, work, off_work, POINT { x: 730, y: 700 }, 24, [true; 4],
    ), None);
    // A free widget must not snap just because it was dragged toward a
    // neighboring monitor earlier; the current pointer and popup are
    // both far from all four candidate edges.
    let inset = RECT { left: 48, top: 0, right: 1920, bottom: 1080 };
    let free_widget = RECT { left: 500, top: 400, right: 720, bottom: 500 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, inset, free_widget, POINT { x: 200, y: 700 }, 24, [true; 4],
    ), None);
    // At an actual free screen edge, retain the original algorithm.
    let free = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let near_left = RECT { left: -5, top: 400, right: 225, bottom: 500 };
    assert_eq!(floating_drop_edge_at_pointer(
        monitor, free, near_left, POINT { x: 15, y: 440 }, 24, [true; 4],
    ), Some(FloatingEdge::Left));
}

#[test]
fn desktop_taskbar_rect_corrects_rcwork_gap_on_every_screen_edge() {
    let monitor = RECT { left: 2048, top: 0, right: 3755, bottom: 960 };
    let bottom = RECT { left: 2048, top: 912, right: 3755, bottom: 960 };
    let work = RECT { left: 2048, top: 0, right: 3755, bottom: 900 };
    assert_eq!(floating_bounds_at_taskbar(monitor, work, &[bottom]).bottom, 912);
    let top = RECT { left: 2048, top: 0, right: 3755, bottom: 48 };
    let top_work = RECT { top: 60, ..monitor };
    assert_eq!(floating_bounds_at_taskbar(monitor, top_work, &[top]).top, 48);
    let left = RECT { left: 2048, top: 0, right: 2096, bottom: 960 };
    let left_work = RECT { left: 2108, ..monitor };
    assert_eq!(floating_bounds_at_taskbar(monitor, left_work, &[left]).left, 2096);
    let right = RECT { left: 3707, top: 0, right: 3755, bottom: 960 };
    let right_work = RECT { right: 3695, ..monitor };
    assert_eq!(floating_bounds_at_taskbar(monitor, right_work, &[right]).right, 3707);
    let other_monitor_bar = RECT { left: 0, top: 1104, right: 2048, bottom: 1152 };
    assert_eq!(floating_bounds_at_taskbar(monitor, work, &[other_monitor_bar]).bottom, 900);
}

#[test]
fn desktop_reset_top_grip_aligns_after_all_previous_docking_states() {
    let body_w = 320;
    let body_h = 90;
    let work = RECT { left: -1920, top: -200, right: 0, bottom: 880 };
    let (target_w, target_h) = floating_reset_top_canvas_size(body_w, body_h);
    let (expanded_x, expanded_y) =
        floating_reset_top_origin(work, target_w, target_h);
    let grip = floating_grip_rect(
        target_w, target_h, Some(FloatingEdge::Top), sc(FLOAT_EDGE_PEEK),
    );
    let (dx, dy) = floating_slide_offset(
        target_w, target_h, FloatingEdge::Top, 0,
    );
    assert_eq!(dx, 0);
    assert_eq!(expanded_x + (grip.left + grip.right) / 2,
        (work.left + work.right) / 2, "the tab remains horizontally centered");

    for old_edge in [
        None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
        Some(FloatingEdge::Top), Some(FloatingEdge::Bottom),
    ] {
        let (old_w, old_h, _) = floating_canvas_geometry(
            body_w, body_h, old_edge,
        );
        // Distinct source geometries are the regression: a free tab was
        // 12px too high, left/right tabs 4px too low, while bottom was OK.
        match old_edge {
            None => assert!(old_h > target_h),
            Some(FloatingEdge::Left | FloatingEdge::Right) =>
                assert!(old_h < target_h),
            Some(FloatingEdge::Top | FloatingEdge::Bottom) =>
                assert_eq!(old_h, target_h),
        }
        assert_eq!(
            floating_reset_top_canvas_size(body_w, body_h),
            (target_w, target_h),
            "reset geometry must be independent of the previous edge: {old_edge:?}",
        );
        for reserved in [false, true] {
            // Physical edges slide the popup HWND. Taskbar/seam edges
            // keep the HWND fixed and shift the layered bitmap instead.
            let origin_y = expanded_y + if reserved { 0 } else { dy };
            let visible_grip_top = origin_y + grip.top
                + if reserved { dy } else { 0 };
            assert_eq!(visible_grip_top, work.top,
                "tab not flush after {old_edge:?}, reserved={reserved}");
        }
        let _ = old_w;
    }

    // Work-area insets and actual taskbar reservations are preserved.
    let monitor = RECT { left: -1920, top: -200, right: 0, bottom: 880 };
    let tiny_inset = RECT { top: -194, ..monitor };
    assert_eq!(
        floating_bounds_at_taskbar(monitor, tiny_inset, &[]).top,
        tiny_inset.top,
        "reset must not change global work-area boundaries",
    );
    let top_taskbar = RECT {
        left: monitor.left, top: monitor.top,
        right: monitor.right, bottom: monitor.top + 48,
    };
    let real_work = RECT { top: monitor.top + 54, ..monitor };
    assert_eq!(
        floating_bounds_at_taskbar(monitor, real_work, &[top_taskbar]).top,
        top_taskbar.bottom,
    );
}

#[test]
fn desktop_taskbar_and_physical_edges_use_identical_slide_offsets() {
    let bounds = RECT { left: -200, top: 60, right: 1100, bottom: 850 };
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        let (w, h, body) = floating_canvas_geometry(240, 72, Some(edge));
        let grip = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
        let (expanded_x, expanded_y) = floating_edge_position(
            bounds, 450, 350, w, h, Some(edge), false, sc(FLOAT_EDGE_PEEK),
        );
        let (collapsed_x, collapsed_y) = floating_edge_position(
            bounds, 450, 350, w, h, Some(edge), true, sc(FLOAT_EDGE_PEEK),
        );
        for reveal in [0_u8, 12, 30, 50, 70, 91, 100] {
            let (dx, dy) = floating_slide_offset(w, h, edge, reveal);
            let fraction = f64::from(reveal) / 100.0;
            let physical_x = collapsed_x
                + (f64::from(expanded_x - collapsed_x) * fraction).round() as i32;
            let physical_y = collapsed_y
                + (f64::from(expanded_y - collapsed_y) * fraction).round() as i32;
            assert_eq!((expanded_x + dx, expanded_y + dy), (physical_x, physical_y),
                "all dock edges must have the same slide travel and easing");
            let moving_tab = floating_slid_canvas_rect(grip, w, h, edge, reveal);
            assert_eq!(moving_tab.left, (grip.left + dx).clamp(0, w));
            assert_eq!(moving_tab.top, (grip.top + dy).clamp(0, h));
            let body_visible = floating_slid_canvas_rect(body, w, h, edge, reveal);
            if reveal == 0 {
                assert!(body_visible.right == body_visible.left
                    || body_visible.top == body_visible.bottom,
                    "the panel must be completely out of the work area at rest");
                match edge {
                    FloatingEdge::Left => {
                        assert_eq!(expanded_x, bounds.left);
                        assert_eq!(expanded_x + moving_tab.left, bounds.left);
                    }
                    FloatingEdge::Right => {
                        assert_eq!(expanded_x + w, bounds.right);
                        assert_eq!(expanded_x + moving_tab.right, bounds.right);
                    }
                    FloatingEdge::Top => {
                        assert_eq!(expanded_y, bounds.top);
                        assert_eq!(expanded_y + moving_tab.top, bounds.top);
                    }
                    FloatingEdge::Bottom => {
                        assert_eq!(expanded_y + h, bounds.bottom);
                        assert_eq!(expanded_y + moving_tab.bottom, bounds.bottom);
                    }
                }
            }
        }
    }
}

#[test]
fn desktop_taskbar_bitmap_slide_is_exact_translation_clipped_on_all_four_sides() {
    // Every colored pixel moves precisely as it would when the HWND itself
    // slides beyond a physical screen edge. Work-area pixels alone survive.
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        let (w, h, _) = floating_canvas_geometry(84, 42, Some(edge));
        let original: Vec<u32> = (0..w*h).map(|i| 0xAA000000 | i as u32).collect();
        for reveal in [0_u8, 15, 40, 60, 85, 100] {
            let mut pixels = original.clone();
            floating_slide_layered_bitmap(&mut pixels, w, h, edge, reveal);
            let (dx, dy) = floating_slide_offset(w, h, edge, reveal);
            for y in 0..h {
                for x in 0..w {
                    let src_x = x - dx;
                    let src_y = y - dy;
                    let expected = if src_x >= 0 && src_x < w
                        && src_y >= 0 && src_y < h {
                        original[(src_y*w + src_x) as usize]
                    } else { 0 };
                    assert_eq!(pixels[(y*w + x) as usize], expected,
                        "taskbar edge must be the same slide, only alpha-clipped");
                }
            }
        }
    }
}

#[test]
fn desktop_taskbar_tab_grab_anchor_stays_fixed_when_virtual_slide_becomes_real() {
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        let (w, h, _) = floating_canvas_geometry(240, 72, Some(edge));
        let grip = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
        let hwnd_origin = POINT { x: 200, y: 400 };
        for reveal in [0_u8, 25, 50, 85, 100] {
            let (dx, dy) = floating_slide_offset(w, h, edge, reveal);
            let anchor = POINT {
                x: (grip.left + grip.right) / 2 + dx,
                y: (grip.top + grip.bottom) / 2 + dy,
            };
            let cursor = POINT {
                x: hwnd_origin.x + anchor.x,
                y: hwnd_origin.y + anchor.y,
            };
            // This is the rebasing performed on the very first drag frame.
            let new_anchor = POINT { x: anchor.x - dx, y: anchor.y - dy };
            let new_origin = POINT {
                x: cursor.x - new_anchor.x,
                y: cursor.y - new_anchor.y,
            };
            assert_eq!(new_origin, POINT {
                x: hwnd_origin.x + dx, y: hwnd_origin.y + dy,
            });
            assert_eq!(new_origin.x + (grip.left + grip.right) / 2, cursor.x);
            assert_eq!(new_origin.y + (grip.top + grip.bottom) / 2, cursor.y);
        }
    }
}

#[test]
fn desktop_bottom_taskbar_docks_above_taskbar_without_sliding_into_it() {
    let monitor = RECT { left: -1920, top: -100, right: 0, bottom: 980 };
    let work = RECT { left: -1920, top: -100, right: 0, bottom: 930 };
    let bounds = floating_usable_bounds(monitor, work);
    assert_eq!(bounds.bottom, work.bottom);
    assert!(floating_edge_is_reserved(monitor, bounds, FloatingEdge::Bottom));
    let (cw, ch, body) = floating_canvas_geometry(240, 168, Some(FloatingEdge::Bottom));
    let (_, y) = floating_edge_position(
        bounds, -600, 850, cw, ch, Some(FloatingEdge::Bottom),
        false, sc(FLOAT_EDGE_PEEK),
    );
    assert_eq!(y + ch, work.bottom);
    assert!(y + body.bottom <= work.bottom);
    // The stationary HWND is kept out of the taskbar, while translated
    // foreground and backdrop pixels are clipped at its real boundary.
}

#[test]
fn desktop_taskbar_dock_respects_top_right_and_left_work_boundaries() {
    let monitor = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    for (work, edge) in [
        (RECT { left: 0, top: 48, right: 1920, bottom: 1080 }, FloatingEdge::Top),
        (RECT { left: 0, top: 0, right: 1860, bottom: 1080 }, FloatingEdge::Right),
        (RECT { left: 55, top: 0, right: 1920, bottom: 1080 }, FloatingEdge::Left),
    ] {
        let bounds = floating_usable_bounds(monitor, work);
        assert!(floating_edge_is_reserved(monitor, bounds, edge));
        let (w, h, _) = floating_canvas_geometry(240, 80, Some(edge));
        let (x, y) = floating_edge_position(bounds, 900, 300, w, h,
            Some(edge), false, sc(FLOAT_EDGE_PEEK));
        assert!(x >= work.left && y >= work.top);
        assert!(x + w <= work.right && y + h <= work.bottom);
        match edge {
            FloatingEdge::Left => assert_eq!(x, work.left),
            FloatingEdge::Right => assert_eq!(x + w, work.right),
            FloatingEdge::Top => assert_eq!(y, work.top),
            FloatingEdge::Bottom => unreachable!(),
        }
    }
    assert!(!floating_edge_is_reserved(monitor, monitor, FloatingEdge::Bottom));
    assert_eq!(floating_usable_bounds(monitor,
        RECT { left: -100, top: 0, right: 1920, bottom: 1080 }).left, monitor.left);
}

#[test]
fn desktop_docked_hover_envelope_is_stable_at_extreme_monitor_pixels() {
    // Simulate cursor at the first/last physical pixel of the selected
    // monitor, including on a seam where the other screen continues.
    // The body can move/clip but this envelope is independent of reveal.
    let bounds = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        let (w, h, body) = floating_canvas_geometry(240, 72, Some(edge));
        let hover = floating_docked_hover_rect_for_geometry(
            bounds, 700, 450, w, h, body, edge,
        );
        let (dock_x, dock_y) = floating_edge_position(
            bounds, 700, 450, w, h, Some(edge), false, sc(FLOAT_EDGE_PEEK),
        );
        let expanded_body = RECT {
            left: dock_x + body.left,
            right: dock_x + body.right,
            top: dock_y + body.top,
            bottom: dock_y + body.bottom,
        };
        let grip = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
        let (dx, dy) = floating_slide_offset(w, h, edge, 0);
        let tab_center = POINT {
            x: dock_x + (grip.left + grip.right) / 2 + dx,
            y: dock_y + (grip.top + grip.bottom) / 2 + dy,
        };
        assert!(point_in_rect(hover, tab_center.x, tab_center.y));
        let panel_corner = POINT {
            x: match edge {
                FloatingEdge::Right => bounds.right - 1,
                _ => expanded_body.left,
            },
            y: match edge {
                FloatingEdge::Bottom => bounds.bottom - 1,
                _ => expanded_body.top,
            },
        };
        assert!(point_in_rect(hover, panel_corner.x, panel_corner.y),
            "an alpha-transparent rounded corner at the screen boundary must retain hover");
        let outside_monitor = match edge {
            FloatingEdge::Left => POINT { x: bounds.left - 1, y: tab_center.y },
            FloatingEdge::Right => POINT { x: bounds.right, y: tab_center.y },
            FloatingEdge::Top => POINT { x: tab_center.x, y: bounds.top - 1 },
            FloatingEdge::Bottom => POINT { x: tab_center.x, y: bounds.bottom },
        };
        assert!(!point_in_rect(hover, outside_monitor.x, outside_monitor.y),
            "hover must not leak into the adjacent screen");
        assert!(hover.left >= bounds.left && hover.top >= bounds.top);
        assert!(hover.right <= bounds.right && hover.bottom <= bounds.bottom);
        // Move strictly beyond the 3px inward tolerance, in the
        // direction perpendicular to the dock line, to request collapse.
        let beyond = POINT {
            x: match edge {
                FloatingEdge::Left => hover.right.saturating_add(1),
                FloatingEdge::Right => hover.left.saturating_sub(1),
                _ => tab_center.x,
            },
            y: match edge {
                FloatingEdge::Top => hover.bottom.saturating_add(1),
                FloatingEdge::Bottom => hover.top.saturating_sub(1),
                _ => tab_center.y,
            },
        };
        assert!(!point_in_rect(hover, beyond.x, beyond.y),
            "a real departure must still be allowed to collapse");
    }
}

#[test]
fn desktop_docked_hover_respects_taskbar_work_area_and_partial_slide() {
    let monitor = RECT { left: 2048, top: 0, right: 3755, bottom: 960 };
    let work = RECT { left: 2048, top: 0, right: 3755, bottom: 912 };
    let (w, h, body) = floating_canvas_geometry(240, 72, Some(FloatingEdge::Bottom));
    let hover = floating_docked_hover_rect_for_geometry(
        work, 2550, 900, w, h, body, FloatingEdge::Bottom,
    );
    assert_eq!(hover.bottom, work.bottom);
    let point = POINT { x: (hover.left + hover.right) / 2, y: 911 };
    assert!(point_in_rect(hover, point.x, point.y));
    assert!(!point_in_rect(hover, point.x, 912),
        "hover must not extend into Explorer's reserved taskbar");
    for reveal in [0_u8, 10, 35, 65, 90, 100] {
        // Current opaque pixels translate; the logical retention area
        // remains exactly the same across all animation frames.
        let (_, dy) = floating_slide_offset(w, h, FloatingEdge::Bottom, reveal);
        let moving = floating_slid_canvas_rect(
            body, w, h, FloatingEdge::Bottom, reveal,
        );
        assert!(moving.top + dy >= body.top.saturating_sub(h));
        assert_eq!(floating_docked_hover_rect_for_geometry(
            work, 2550, 900, w, h, body, FloatingEdge::Bottom,
        ), hover);
    }
    // One-pixel work-area boundaries on taskbar-less physical edges
    // must be included too.
    assert!(monitor.right > hover.right);
}

#[test]
fn desktop_shared_seam_docks_to_monitor_under_cursor_and_auto_hides() {
    // Left monitor ends where right monitor starts (x=0); when the
    // cursor is on the left side the chosen edge is Right, on the
    // other monitor it is Left. The rendered grip never crosses x=0.
    let left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
    let right = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
    for (monitor, widget, pointer, edge) in [
        (
            left, RECT { left: -180, top: 460, right: 18, bottom: 550 },
            POINT { x: -2, y: 505 }, FloatingEdge::Right,
        ),
        (
            right, RECT { left: -10, top: 460, right: 210, bottom: 550 },
            POINT { x: 3, y: 505 }, FloatingEdge::Left,
        ),
    ] {
        assert_eq!(floating_drop_edge_at_pointer(
            monitor, monitor, widget, pointer, 24, [true; 4],
        ), Some(edge), "release screen alone selects docking monitor");
        assert!(floating_edge_needs_clip(monitor, monitor, edge, true),
            "shared seam must alpha-clip the popup into the mouse screen");
        let (w, h, body) = floating_canvas_geometry(240, 72, Some(edge));
        let (expanded_x, expanded_y) = floating_edge_position(
            monitor, pointer.x, pointer.y, w, h, Some(edge), false, sc(FLOAT_EDGE_PEEK),
        );
        let tab = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
        let collapsed = floating_slid_canvas_rect(tab, w, h, edge, 0);
        let panel = floating_slid_canvas_rect(body, w, h, edge, 0);
        assert!(panel.right == panel.left || panel.bottom == panel.top,
            "panel must fully retract at the shared seam");
        let grip_x = match edge {
            FloatingEdge::Right => expanded_x + collapsed.right,
            FloatingEdge::Left => expanded_x + collapsed.left,
            _ => unreachable!(),
        };
        assert_eq!(grip_x, 0, "retracted tab must be flush with shared seam");
        assert!(expanded_y >= monitor.top && expanded_y + h <= monitor.bottom);
        assert!(expanded_x >= monitor.left && expanded_x + w <= monitor.right);
    }
}

#[test]
fn desktop_shared_seam_dock_clips_sliding_pixels_inside_chosen_monitor() {
    let monitor_left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
    let monitor_right = RECT { left: 0, top: 0, right: 2560, bottom: 1440 };
    for (monitor, edge) in [
        (monitor_left, FloatingEdge::Right),
        (monitor_right, FloatingEdge::Left),
    ] {
        let (w, h, _) = floating_canvas_geometry(240, 72, Some(edge));
        let (x, y) = floating_edge_position(
            monitor, monitor.left, 360, w, h, Some(edge), false, sc(FLOAT_EDGE_PEEK),
        );
        let source = vec![0xFFFFFFFF; (w * h) as usize];
        for reveal in [0_u8, 15, 50, 85, 100] {
            let mut bitmap = source.clone();
            floating_slide_layered_bitmap(&mut bitmap, w, h, edge, reveal);
            let (dx, dy) = floating_slide_offset(w, h, edge, reveal);
            assert!(bitmap.iter().any(|&pixel| pixel != 0),
                "drag tab must remain visible even when panel is hidden");
            for py in 0..h {
                for px in 0..w {
                    let sx = px - dx;
                    let sy = py - dy;
                    let pixel = bitmap[(py * w + px) as usize];
                    assert_eq!(pixel != 0, sx >= 0 && sx < w && sy >= 0 && sy < h);
                    assert!(x + px >= monitor.left && x + px < monitor.right);
                    assert!(y + py >= monitor.top && y + py < monitor.bottom);
                }
            }
        }
    }
    // Independent external edges need no clipping, unless Explorer
    // reserves that edge for its own taskbar.
    assert!(!floating_edge_needs_clip(
        monitor_right, monitor_right, FloatingEdge::Top, false,
    ));
    assert!(floating_edge_needs_clip(
        monitor_right, RECT { top: 48, ..monitor_right }, FloatingEdge::Top, false,
    ));
}

#[test]
fn desktop_left_taskbar_dock_never_crosses_work_area() {
    let monitor = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
    let work = RECT { left: -1872, top: 0, right: 0, bottom: 1040 };
    let bounds = floating_usable_bounds(monitor, work);
    assert_eq!(bounds.left, work.left);
    let (width, height, _) = floating_canvas_geometry(
        240, 168, Some(FloatingEdge::Left));
    let (x, y) = floating_edge_position(
        bounds, -1860, 140, width, height,
        Some(FloatingEdge::Left), false, sc(FLOAT_EDGE_PEEK));
    assert_eq!(x, work.left);
    assert!(y >= work.top && y + height <= work.bottom);
    assert!(floating_edge_is_reserved(monitor, bounds, FloatingEdge::Left));
}

#[test]
fn desktop_release_tab_switch_preserves_body_screen_origin() {
    let body_w = 240;
    let body_h = 150;
    let (old_w, old_h, old_body) =
        floating_canvas_geometry(body_w, body_h, Some(FloatingEdge::Left));
    let _ = (old_w, old_h);
    let (new_w, new_h, new_body) =
        floating_canvas_geometry(body_w, body_h, None);
    let _ = (new_w, new_h);
    let old_x = 500;
    let old_y = 250;
    let final_x = old_x + old_body.left - new_body.left;
    let final_y = old_y + old_body.top - new_body.top;
    assert_eq!(final_x + new_body.left, old_x + old_body.left);
    assert_eq!(final_y + new_body.top, old_y + old_body.top);
}

#[test]
fn desktop_drag_drop_recognizes_partially_offscreen_old_canvas() {
    let bounds = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    let still_peeking = RECT { left: -200, top: 300, right: 20, bottom: 400 };
    assert_eq!(floating_drop_edge(bounds, still_peeking, 24, [true; 4]), Some(FloatingEdge::Left));
    let free = RECT { left: 640, top: 300, right: 860, bottom: 400 };
    assert_eq!(floating_drop_edge(bounds, free, 24, [true; 4]), None);
}

#[test]
fn desktop_tab_is_outside_taskbar_sized_panel() {
    let body_w = 240;
    let body_h = 168;
    for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                 Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
        let (width, height, body) = floating_canvas_geometry(body_w, body_h, edge);
        let tab = floating_grip_rect(width, height, edge, sc(FLOAT_EDGE_PEEK));
        assert_eq!(body.right - body.left, body_w);
        assert_eq!(body.bottom - body.top, body_h);
        assert!(tab.right <= body.left || tab.left >= body.right
            || tab.bottom <= body.top || tab.top >= body.bottom);
        assert!(tab.left >= 0 && tab.top >= 0
            && tab.right <= width && tab.bottom <= height);
    }
}

#[test]
fn desktop_collapsed_bitmap_only_keeps_grip_pixels_visible() {
    let w = 240;
    let h = 168;
    let style = ThemeStyle::dark_default();
    for edge in [None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
                 Some(FloatingEdge::Top), Some(FloatingEdge::Bottom)] {
        let mut pixels = vec![0u32; (w * h) as usize];
        render_floating_handle_bitmap(&mut pixels, w, h, edge, false, &style, 100);
        let r = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
        for y in 0..h {
            for x in 0..w {
                let pixel = pixels[(y * w + x) as usize];
                if !point_in_rect(r, x, y) { assert_eq!(pixel, 0); }
            }
        }
        assert_ne!(pixels[(((r.top + r.bottom) / 2) * w +
            (r.left + r.right) / 2) as usize], 0);
    }
}


#[test]
fn v24_27_real_alpha_zero_seam_receives_hover_recheck_on_every_edge() {
    // V24.26 missed this case: the nominal rectangles are adjacent,
    // and the *two transparent contour pixels are INSIDE those RECTs*.
    // The rectangle-only gap test therefore returns false, while Windows
    // may issue WM_MOUSELEAVE before the cursor reaches the visible tab.
    let inset = sc(1).max(1);
    for (bw, bh) in [(240, 42), (48, 168)] {
        for edge in [
            FloatingEdge::Left, FloatingEdge::Right,
            FloatingEdge::Top, FloatingEdge::Bottom,
        ] {
            let (_, _, body) = floating_canvas_geometry(bw, bh, Some(edge));
            let (canvas_w, canvas_h, _) =
                floating_canvas_geometry(bw, bh, Some(edge));
            let grip = floating_grip_rect(
                canvas_w, canvas_h, Some(edge), sc(FLOAT_EDGE_PEEK),
            );
            let (body_x, body_y, grip_x, grip_y) = match edge {
                FloatingEdge::Left => (
                    body.right - 1, (grip.top + grip.bottom) / 2,
                    grip.left, (grip.top + grip.bottom) / 2,
                ),
                FloatingEdge::Right => (
                    body.left, (grip.top + grip.bottom) / 2,
                    grip.right - 1, (grip.top + grip.bottom) / 2,
                ),
                FloatingEdge::Top => (
                    (grip.left + grip.right) / 2, body.bottom - 1,
                    (grip.left + grip.right) / 2, grip.top,
                ),
                FloatingEdge::Bottom => (
                    (grip.left + grip.right) / 2, body.top,
                    (grip.left + grip.right) / 2, grip.bottom - 1,
                ),
            };
            assert_eq!(match edge {
                FloatingEdge::Left => body.right,
                FloatingEdge::Right => body.left,
                FloatingEdge::Top => body.bottom,
                FloatingEdge::Bottom => body.top,
            }, match edge {
                FloatingEdge::Left => grip.left,
                FloatingEdge::Right => grip.right,
                FloatingEdge::Top => grip.top,
                FloatingEdge::Bottom => grip.bottom,
            }, "the geometric rectangles must touch, not leave a gap");
            let body_w = body.right - body.left;
            let body_h = body.bottom - body.top;
            let grip_w = grip.right - grip.left;
            let grip_h = grip.bottom - grip.top;
            for (x, y, rect, w, h) in [
                (body_x, body_y, body, body_w, body_h),
                (grip_x, grip_y, grip, grip_w, grip_h),
            ] {
                assert!(point_in_rect(rect, x, y),
                    "native bounds label the pixel as inside the panel/tab");
                assert!(!panel_outer_shape_contains(
                    w, h, x - rect.left, y - rect.top, 8,
                ), "actual shape paints alpha zero at this boundary pixel");
                assert!(!floating_transition_corridor_contains(
                    body, grip, edge, x, y,
                ), "old V24.26 transition incorrectly ignored these pixels");
                assert!(floating_transparent_panel_grip_seam_contains(
                    body, grip, edge, x, y,
                ), "actual transparent seam needs timed mouse hover recovery");
                assert_eq!(floating_drag_tab_hit_code(body, grip, x, y), 1,
                    "the existing geometrical hit code remains unchanged");
            }
            // Neighboring visible pixels and other halo gutters must
            // never be treated as a transition into the drag tab.
            let (x, y) = match edge {
                FloatingEdge::Left => (body.right - inset - 1, body_y),
                FloatingEdge::Right => (body.left + inset, body_y),
                FloatingEdge::Top => (body_x, body.bottom - inset - 1),
                FloatingEdge::Bottom => (body_x, body.top + inset),
            };
            assert!(!floating_transparent_panel_grip_seam_contains(
                body, grip, edge, x, y,
            ));
            assert!(!floating_transparent_panel_grip_seam_contains(
                body, grip, edge, 0, 0,
            ), "faraway transparent halo gutter must stay unaffected");
        }
    }
}

#[test]
fn v24_26_direct_body_to_grip_uses_visual_corridor_without_enlarging_drag_hit() {
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        for (body_width, body_height) in [(240, 42), (48, 168)] {
            let (w, h, body) = floating_canvas_geometry(
                body_width, body_height, Some(edge),
            );
            let grip = floating_grip_rect(w, h, Some(edge), sc(FLOAT_EDGE_PEEK));
            let transition = floating_hover_probe_rect(body, grip, Some(edge));
            let mut gap_count = 0;
            for y in transition.top.max(0)..transition.bottom.min(h) {
                for x in transition.left.max(0)..transition.right.min(w) {
                    if floating_transition_corridor_contains(body, grip, edge, x, y) {
                        gap_count += 1;
                        assert_eq!(floating_drag_tab_hit_code(body, grip, x, y), -1,
                            "gutter is only a hover transition, never a draggable region");
                    }
                }
            }
            if matches!(edge, FloatingEdge::Right | FloatingEdge::Bottom) {
                assert!(gap_count > 0,
                    "the offset between body and tab must be protected on this edge");
            }
            let center_x = (grip.left + grip.right) / 2;
            let center_y = (grip.top + grip.bottom) / 2;
            assert!(!floating_transition_corridor_contains(
                body, grip, edge, center_x, center_y,
            ), "real tab is never mistaken for transition gutter");
            assert!(floating_grip_hit_allowed(Some(edge), false, false, 0),
                "an expanded docked grip may trigger glow before fade-in progresses");
            assert!(!floating_grip_hit_allowed(Some(edge), true, false, 0),
                "normal auto-collapse must keep its original behavior");
            assert!(!floating_grip_hit_allowed(None, false, false, 0),
                "a free floating hidden tab must not become a new hit target");
            assert!(!floating_grip_hit_allowed(Some(edge), false, true, 0),
                "a collapsed tab must not use expanded-panel recovery logic");
            assert!(floating_grip_hit_allowed(Some(edge), false, false, 1));
        }
    }
}

#[test]
fn frosted_backdrop_is_prepared_before_panel_alpha_becomes_translucent() {
    let mut style = ThemeStyle::dark_default();
    assert!(!panel_blur_backdrop_needed(&style));
    style.panel_frosted_strength = 40;
    let original = style.color(StyleColorTarget::PanelBackground);
    let opaque = Color::rgba(original.r, original.g, original.b, 255);
    style.set_color(StyleColorTarget::PanelBackground, opaque);
    assert!(!style.panel_frosted_active());
    assert!(panel_blur_backdrop_needed(&style));
    style.set_color(
        StyleColorTarget::PanelBackground,
        Color::rgba(original.r, original.g, original.b, 254),
    );
    assert!(style.panel_frosted_active());
    assert!(panel_blur_backdrop_needed(&style));
    style.panel_frosted_strength = 0;
    assert!(!panel_blur_backdrop_needed(&style));
}

#[test]
fn expanded_non_collapsing_dock_keeps_cursor_watch_for_every_edge() {
    for edge in [
        FloatingEdge::Top, FloatingEdge::Bottom,
        FloatingEdge::Left, FloatingEdge::Right,
    ] {
        assert!(docked_grip_needs_hover_watch(Some(edge), false, false, false));
        assert!(!docked_grip_needs_hover_watch(Some(edge), true, false, false));
        assert!(!docked_grip_needs_hover_watch(Some(edge), false, true, false));
        assert!(!docked_grip_needs_hover_watch(Some(edge), false, false, true));
    }
    assert!(!docked_grip_needs_hover_watch(None, false, false, false));
    assert!(!floating_grip_should_show(Some(FloatingEdge::Top), false, false, false));
}

#[test]
fn frosted_grip_backdrop_and_foreground_share_the_same_fade_alpha() {
    for animated_alpha in [0, 1, 25, 50, 99, 100] {
        assert_eq!(
            floating_grip_visual_alpha(true, false, animated_alpha),
            animated_alpha,
            "both visual layers must fade when docked auto-collapse is off",
        );
        assert_eq!(
            floating_grip_visual_alpha(false, false, animated_alpha),
            100,
            "auto-collapsing dock retains an exposed drag tab",
        );
        assert_eq!(
            floating_grip_visual_alpha(true, true, animated_alpha),
            100,
            "dragging keeps the grip and its backing visible",
        );
    }
    assert_eq!(floating_grip_visual_alpha(true, false, 255), 100);
}

#[test]
fn docked_expanded_grip_fades_like_free_desktop_without_changing_auto_hide() {
    assert!(floating_grip_should_show(None, false, true, false));
    assert!(!floating_grip_should_show(None, false, false, false));
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        assert!(!floating_grip_should_show(Some(edge), false, false, false),
            "expanded docked desktop must hide its grip off-hover");
        assert!(floating_grip_should_show(Some(edge), false, true, false));
        assert!(floating_grip_should_show(Some(edge), false, false, true),
            "grip must remain visible for the duration of a drag");
        assert!(floating_grip_should_show(Some(edge), true, false, false),
            "legacy auto-collapsed edge grip must stay visible");
    }
}

#[test]
fn desktop_reset_top_center_respects_monitor_bounds_and_edges() {
    let primary = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    assert_eq!(floating_reset_top_origin(primary, 320, 110), (800, 0));
    let left = RECT { left: -2560, top: -100, right: -640, bottom: 980 };
    assert_eq!(floating_reset_top_origin(left, 320, 110), (-1760, -100));
    let above = RECT { left: 0, top: -1200, right: 1600, bottom: -120 };
    assert_eq!(floating_reset_top_origin(above, 320, 110), (640, -1200));
    let narrow = RECT { left: 1920, top: 40, right: 2130, bottom: 440 };
    assert_eq!(floating_reset_top_origin(narrow, 320, 110), (1920, 40));
    assert_eq!(
        initial_floating_visibility(Some(FloatingEdge::Top), DisplayPosition::Desktop, true),
        (true, 0),
        "reset must publish the already-collapsed grip without a flash",
    );
    assert_eq!(
        initial_floating_visibility(Some(FloatingEdge::Top), DisplayPosition::Desktop, false),
        (false, 100),
        "desktop with auto-collapse disabled must stay expanded",
    );
}

#[test]
fn docked_desktop_first_frame_is_already_fully_collapsed() {
    for edge in [
        FloatingEdge::Left, FloatingEdge::Right,
        FloatingEdge::Top, FloatingEdge::Bottom,
    ] {
        assert_eq!(
            initial_floating_visibility(Some(edge), DisplayPosition::Desktop, true),
            (true, 0),
            "edge-docked desktop must never publish an expanded first frame"
        );
        assert_eq!(
            initial_floating_visibility(Some(edge), DisplayPosition::Desktop, false),
            (false, 100),
            "disabling automatic collapse must leave the body visible"
        );
        assert_eq!(
            initial_floating_visibility(Some(edge), DisplayPosition::Taskbar, false),
            (true, 0),
            "taskbar-space fallback still collapses automatically"
        );
    }
    assert_eq!(
        initial_floating_visibility(None, DisplayPosition::Desktop, true),
        (false, 100),
        "an undocked desktop must stay fully expanded"
    );
}

#[test]
fn desktop_auto_collapse_defaults_enabled_round_trips_and_respects_mode() {
    let legacy: SettingsFile = serde_json::from_str("{}").unwrap();
    assert!(legacy.desktop_auto_collapse);
    let disabled = SettingsFile {
        display_position: DisplayPosition::Desktop,
        desktop_auto_collapse: false,
        ..Default::default()
    };
    let parsed: SettingsFile =
        serde_json::from_str(&serde_json::to_string(&disabled).unwrap()).unwrap();
    assert!(!parsed.desktop_auto_collapse);
    assert!(!floating_auto_collapse_allowed(DisplayPosition::Desktop, false));
    assert!(floating_auto_collapse_allowed(DisplayPosition::Desktop, true));
    assert!(floating_auto_collapse_allowed(DisplayPosition::Taskbar, false),
        "taskbar-space fallback keeps its existing automatic collapse behavior");
}

#[test]
fn desktop_cross_monitor_drag_keeps_pointer_anchor_at_each_dpi() {
    // Same logical grip point at 100%, 150% and 200% scaling. Cursor
    // coordinates are absolute virtual-desktop pixels, including a
    // monitor to the left of the primary.
    for pointer in [
        POINT { x: -8, y: 530 },
        POINT { x: 0, y: 530 },
        POINT { x: 1920, y: 530 },
        POINT { x: 3700, y: -120 },
    ] {
        for (from_dpi, to_dpi) in [
            (96, 96), (96, 144), (144, 96), (96, 192), (192, 144),
        ] {
            for anchor in [0, 8, 25, 110, -16] {
                let new_anchor = floating_drag_anchor_for_dpi(
                    anchor, from_dpi, to_dpi,
                );
                let new_origin = pointer.x - new_anchor;
                assert_eq!(new_origin + new_anchor, pointer.x,
                    "drag must stay attached across the seam");
                let expected = (f64::from(anchor) * f64::from(to_dpi)
                    / f64::from(from_dpi)).round() as i32;
                assert_eq!(new_anchor, expected,
                    "drag-grip location must scale to the target monitor");
            }
        }
    }
    assert_eq!(floating_drag_anchor_for_dpi(25, 96, 144), 38);
    assert_eq!(floating_drag_anchor_for_dpi(-25, 96, 144), -38);
    assert_eq!(floating_drag_anchor_for_dpi(30, 144, 96), 20);
}

#[test]
fn free_grip_moves_below_near_top_without_pushing_body_or_docking() {
    let bounds = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    for (body_width, body_height) in [(240, 42), (48, 168)] {
        let (w, h, top_body) =
            floating_free_canvas_geometry(body_width, body_height, false);
        let (bw, bh, bottom_body) =
            floating_free_canvas_geometry(body_width, body_height, true);
        assert_eq!((w, h), (bw, bh), "switch must not resize the HWND");
        let top_grip = floating_layout_grip_rect(w, h, None, false);
        let bottom_grip = floating_layout_grip_rect(w, h, None, true);
        assert_eq!(top_grip.bottom, top_body.top);
        assert_eq!(bottom_grip.top, bottom_body.bottom);

        let body_x = bounds.left + 400;
        let body_y = bounds.top + 5;
        assert!(floating_choose_free_grip_below(bounds, body_y, body_height));
        // A previously below-body tab can sit 5px below the top
        // without moving the real panel or producing a false dock.
        // A top tab actually crossing this edge is tested BEFORE flipping.
        let (x, y) = floating_clamp_drag_visible_position(
            bounds, w, h, bottom_body, None, true,
            body_x - bottom_body.left, body_y - bottom_body.top,
        );
        assert_eq!(y + bottom_body.top, body_y);
        assert_eq!(x + bottom_body.left, body_x);
        let pointer = POINT { x: body_x + body_width / 2, y: body_y + 2 };
        assert_eq!(floating_desktop_contact_edge(
            bounds, w, h, bottom_body, None, true, POINT { x, y }, pointer,
        ), None, "a free tab switch is not a top-edge dock");

        // At the real top boundary, contact still docks normally.
        let (x, y) = floating_clamp_drag_visible_position(
            bounds, w, h, bottom_body, None, true,
            body_x - bottom_body.left, bounds.top - bottom_body.top,
        );
        assert_eq!(floating_desktop_contact_edge(
            bounds, w, h, bottom_body, None, true, POINT { x, y }, pointer,
        ), Some(FloatingEdge::Top));

        // When an already-below tab reaches the taskbar, the REAL
        // bottom grip contact still qualifies even though the next
        // free layout would prefer the tab above the body.
        let tab_touch_y = bounds.bottom - body_height - sc(FLOAT_EDGE_PEEK);
        assert!(!floating_choose_free_grip_below(bounds, tab_touch_y, body_height));
        let (old_x, old_y) = floating_clamp_drag_visible_position(
            bounds, w, h, bottom_body, None, true,
            body_x - bottom_body.left, tab_touch_y - bottom_body.top,
        );
        assert_eq!(floating_desktop_contact_edge(
            bounds, w, h, bottom_body, None, true,
            POINT { x: old_x, y: old_y }, pointer,
        ), Some(FloatingEdge::Bottom));
        // Moving near the lower taskbar keeps the standard top tab.
        let near_bottom = bounds.bottom - body_height - 5;
        assert!(!floating_choose_free_grip_below(bounds, near_bottom, body_height));
        let (x, y) = floating_clamp_drag_visible_position(
            bounds, w, h, top_body, None, false,
            body_x - top_body.left, near_bottom - top_body.top,
        );
        assert_eq!(y + top_body.top, near_bottom);
        assert_eq!(floating_desktop_contact_edge(
            bounds, w, h, top_body, None, false, POINT { x, y }, pointer,
        ), None);
    }
    let legacy: SettingsFile = serde_json::from_str("{}").unwrap();
    assert!(!legacy.floating_free_grip_below);
    let current = SettingsFile {
        floating_edge: None, floating_free_grip_below: true,
        ..SettingsFile::default()
    };
    let restored: SettingsFile =
        serde_json::from_str(&serde_json::to_string(&current).unwrap()).unwrap();
    assert!(restored.floating_free_grip_below,
        "a free tab below the panel must survive restart");
}

#[test]
fn visible_content_geometry_matches_clamp_and_contact_in_all_orientations() {
    let bounds = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    for (body_width, body_height) in [(240, 42), (48, 168)] {
        for edge in [
            None, Some(FloatingEdge::Left), Some(FloatingEdge::Right),
            Some(FloatingEdge::Top), Some(FloatingEdge::Bottom),
        ] {
            for below in [false, true] {
                if edge.is_some() && below { continue; }
                let (w, h, body) = if edge.is_none() {
                    floating_free_canvas_geometry(body_width, body_height, below)
                } else {
                    floating_canvas_geometry(body_width, body_height, edge)
                };
                let grip = floating_layout_grip_rect(w, h, edge, below);
                let visible = floating_visible_content_rect(w, h, body, edge, below);
                assert_eq!(visible.left, body.left.min(grip.left));
                assert_eq!(visible.top, body.top.min(grip.top));
                assert_eq!(visible.right, body.right.max(grip.right));
                assert_eq!(visible.bottom, body.bottom.max(grip.bottom));
                let desired_x = bounds.left - visible.left;
                let desired_y = bounds.top - visible.top;
                let (x, y) = floating_clamp_drag_visible_position(
                    bounds, w, h, body, edge, below, desired_x, desired_y,
                );
                assert_eq!((x, y), (desired_x, desired_y),
                    "clamping must use the same visible bounds as contact");
                assert_eq!(floating_desktop_contact_edge(
                    bounds, w, h, body, edge, below,
                    POINT { x, y }, POINT { x: x + w / 2, y: bounds.top },
                ), Some(FloatingEdge::Top),
                    "top contact remains exact in every orientation");
            }
        }
    }
}

#[test]
fn free_top_grip_contact_docks_before_adaptive_flip() {
    // Reproduction: a free widget dragged upward from the desktop
    // center shows its six-dot tab ABOVE the panel. Once that tab hits
    // the top boundary, it must dock instead of silently flipping below.
    let bounds = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    for (bw, bh) in [(240, 42), (48, 168)] {
        let (w, h, body) = floating_free_canvas_geometry(bw, bh, false);
        let grip = floating_layout_grip_rect(w, h, None, false);
        let origin_x = bounds.left + 400;
        let pointer = POINT { x: origin_x + w / 2, y: bounds.top + 20 };
        let test_release = |grip_top: i32| {
            floating_existing_free_tab_contact(
                bounds, w, h, body, false,
                POINT { x: origin_x, y: grip_top - grip.top }, pointer,
            )
        };
        // Even the last pixel before contact must remain free.
        for gap in [1, 5, 12, 23] {
            assert_eq!(test_release(bounds.top + gap), None,
                "near-top drag tabs must never magnetically dock");
        }
        assert_eq!(test_release(bounds.top), Some(FloatingEdge::Top));
        assert_eq!(test_release(bounds.top - 1), Some(FloatingEdge::Top),
            "a tab crossing the top is clamped to an actual contact");

        let body_top_at_contact = bounds.top + body.top - grip.top;
        assert!(floating_choose_free_grip_below(bounds, body_top_at_contact - 1, bh),
            "the prospective layout would flip, but existing contact wins");

        // If the tab was ALREADY below the body, proximity to the top
        // does not create a top contact; body position is preserved.
        let (_, _, below_body) = floating_free_canvas_geometry(bw, bh, true);
        let below_y = bounds.top + 5 - below_body.top;
        assert_eq!(floating_existing_free_tab_contact(
            bounds, w, h, below_body, true,
            POINT { x: origin_x, y: below_y }, pointer,
        ), None);
        // Prior bottom-taskbar behavior remains intact.
        let bottom_grip = floating_layout_grip_rect(w, h, None, true);
        let bottom_y = bounds.bottom - bottom_grip.bottom;
        assert_eq!(floating_existing_free_tab_contact(
            bounds, w, h, below_body, true,
            POINT { x: origin_x, y: bottom_y }, pointer,
        ), Some(FloatingEdge::Bottom));
    }
}

#[test]
fn explicit_desktop_only_docks_on_actual_visible_contact() {
    let bounds = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    for (body_width, body_height) in [(240, 42), (48, 168)] {
        let (w, h, body) = floating_canvas_geometry(body_width, body_height, None);
        let grip = floating_grip_rect(w, h, None, sc(FLOAT_EDGE_PEEK));
        let (left, right) = (body.left.min(grip.left), body.right.max(grip.right));
        let (top, bottom) = (body.top.min(grip.top), body.bottom.max(grip.bottom));
        let inside_x = bounds.left + 350;
        let inside_y = bounds.top + 250;
        let pointer = POINT { x: bounds.left + 450, y: bounds.top + 300 };
        let drop_edge = |requested_x, requested_y| {
            let (x, y) = floating_clamp_free_visible_position(
                bounds, w, h, body, requested_x, requested_y,
            );
            floating_desktop_contact_edge(
                bounds, w, h, body, None, false, POINT { x, y }, pointer,
            )
        };
        assert_eq!(drop_edge(inside_x, inside_y), None);
        for gap in [1, 5, 12, 23] {
            assert_eq!(drop_edge(bounds.left + gap - left, inside_y), None);
            assert_eq!(drop_edge(bounds.right - gap - right, inside_y), None);
            assert_eq!(drop_edge(inside_x, bounds.top + gap - top), None);
            assert_eq!(drop_edge(inside_x, bounds.bottom - gap - bottom), None);
        }
        assert_eq!(drop_edge(bounds.left - left, inside_y), Some(FloatingEdge::Left));
        assert_eq!(drop_edge(bounds.right - right, inside_y), Some(FloatingEdge::Right));
        assert_eq!(drop_edge(inside_x, bounds.top - top), Some(FloatingEdge::Top));
        assert_eq!(drop_edge(inside_x, bounds.bottom - bottom), Some(FloatingEdge::Bottom));
        // A genuinely overhanging drop is corrected to the boundary.
        assert_eq!(drop_edge(bounds.left - left - 50, inside_y), Some(FloatingEdge::Left));
    }
}

#[test]
fn desktop_drag_retains_actual_tab_side_when_testing_top_and_bottom_contact() {
    // Regression for a TOP-docked widget being dragged with its tab still
    // below the panel. The free canvas would move that tab above the
    // panel, introducing a false top contact and hiding bottom contact.
    let bounds = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    let body_w = 240;
    let body_h = 42;
    let edge = Some(FloatingEdge::Top);
    for detached in [false, true] {
        let (w, h, body) =
            floating_drag_canvas_geometry(body_w, body_h, edge, detached);
        let grip = floating_grip_rect(w, h, edge, sc(FLOAT_EDGE_PEEK));
        let x = bounds.left + 400;
        let pointer = POINT { x: x + w / 2, y: bounds.top + 40 };
        let release = |requested_y: i32| {
            let (cx, cy) = floating_clamp_drag_visible_position(
                bounds, w, h, body, edge, false, x, requested_y,
            );
            floating_desktop_contact_edge(
                bounds, w, h, body, edge, false, POINT { x: cx, y: cy }, pointer,
            )
        };
        // Screenshot 1: the actual panel is still 15px below the top.
        // A hypothetical free tab would extend above it and fabricate
        // a top collision. The real tab is BELOW the panel.
        let above_gap = bounds.top + 15 - body.top;
        assert_eq!(release(above_gap), None,
            "top must not dock before the visible drag canvas reaches it");
        assert_eq!(release(bounds.top - body.top), Some(FloatingEdge::Top));

        // Screenshot 2: the actual bottom tab meets the taskbar's inner
        // boundary, although the panel itself remains one tab away.
        let at_taskbar = bounds.bottom - grip.bottom;
        assert!(at_taskbar + body.bottom < bounds.bottom);
        assert_eq!(release(at_taskbar), Some(FloatingEdge::Bottom),
            "bottom tab contact must dock even though the panel is above it");
        for gap in [1, 5, 12, 23] {
            assert_eq!(release(at_taskbar - gap), None,
                "near the taskbar is not the same as touching it");
        }
        assert_eq!(release(at_taskbar + 10), Some(FloatingEdge::Bottom),
            "an overlapping tab should clamp and register real contact");
    }
}

#[test]
fn taskbar_fallback_retains_legacy_proximity_and_ignores_retired_setting() {
    let monitor = RECT { left: -1920, top: -120, right: 0, bottom: 960 };
    let work = RECT { left: -1920, top: -120, right: 0, bottom: 912 };
    let near_edge = RECT { left: -1908, top: 120, right: -1608, bottom: 220 };
    let pointer = POINT { x: -1899, y: 159 };
    assert_eq!(
        floating_drop_edge_at_pointer(monitor, work, near_edge, pointer, 24, [true; 4]),
        Some(FloatingEdge::Left),
    );
    let mut old = serde_json::to_value(SettingsFile::default()).unwrap();
    old["desktop_snap"] = false.into();
    let restored: SettingsFile = serde_json::from_value(old).unwrap();
    assert!(!serde_json::to_string(&restored).unwrap().contains("desktop_snap"));
}

#[test]
fn free_desktop_can_put_visible_body_and_grip_flush_against_all_edges() {
    // The panel's halo gutter is transparent and must not become a
    // permanent gap. Keep both body and drag tab inside the usable area,
    // including a secondary monitor with negative coordinates.
    let work = RECT { left: -1920, top: -120, right: 0, bottom: 920 };
    for (body_width, body_height) in [(240, 42), (48, 168)] {
        let (width, height, body) =
            floating_canvas_geometry(body_width, body_height, None);
        let grip = floating_grip_rect(width, height, None, sc(FLOAT_EDGE_PEEK));
        let clamp = |x, y| floating_clamp_free_visible_position(
            work, width, height, body, x, y,
        );

        let (left, _) = clamp(-5000, 100);
        assert_eq!(left + body.left, work.left, "panel can touch left edge");
        assert!(left < work.left, "transparent left gutter may be offscreen");

        let (right, _) = clamp(2000, 100);
        assert_eq!(right + body.right, work.right, "panel can touch right edge");

        let (_, top) = clamp(-1000, -5000);
        assert_eq!(top + grip.top, work.top, "drag tab can touch top edge");

        let (_, bottom) = clamp(-1000, 5000);
        assert_eq!(bottom + body.bottom, work.bottom, "panel can touch bottom edge");

        let unchanged = (work.left + 200, work.top + 200);
        assert_eq!(clamp(unchanged.0, unchanged.1), unchanged,
            "interior drop must keep the mouse-relative position");

        for (requested_x, requested_y) in [
            (-5000, -5000), (2000, -5000), (-5000, 5000), (2000, 5000),
        ] {
            let (x, y) = clamp(requested_x, requested_y);
            assert!(x + body.left >= work.left && x + body.right <= work.right);
            assert!(y + body.top >= work.top && y + body.bottom <= work.bottom);
            assert!(x + grip.left >= work.left && x + grip.right <= work.right);
            assert!(y + grip.top >= work.top && y + grip.bottom <= work.bottom);
        }
    }
}

#[test]
fn docked_restore_uses_saved_desktop_monitor_instead_of_dpi_scaled_canvas_center() {
    let left = RECT { left: -1920, top: 0, right: 0, bottom: 1080 };
    let right = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    // A docked popup on the right monitor can have a canvas center
    // crossing the seam after a DPI change or a display rearrangement.
    let origin = (-20, 200);
    let guessed_center = floating_restore_anchor(origin.0, origin.1, 140, 200).unwrap();
    assert!(guessed_center.x >= right.left);
    let anchored = floating_docked_monitor_anchor(
        Some(FloatingEdge::Right), origin.0, origin.1, guessed_center
    );
    assert!(anchored.x < left.right);
    assert_eq!(anchored, POINT { x: -19, y: 201 });
    assert_eq!(
        floating_docked_monitor_anchor(None, origin.0, origin.1, guessed_center),
        guessed_center,
        "free desktop widgets should keep their canvas-center monitor selection",
    );
}

#[test]
fn desktop_restore_prefers_saved_monitor_over_taskbar_and_handles_disconnection() {
    let taskbar = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    let left_screen = RECT { left: -1600, top: -200, right: 0, bottom: 900 };
    let right_screen = RECT { left: 1920, top: 0, right: 3840, bottom: 1080 };
    let resolve = |p: POINT| {
        [left_screen, taskbar, right_screen].into_iter().find(|r| {
            p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom
        })
    };
    // Negative virtual-screen coordinates and different taskbar screens
    // must not override the saved desktop display.
    let left = floating_restore_anchor(-1550, 120, 280, 90);
    assert_eq!(floating_restore_bounds(left, taskbar, resolve), left_screen);
    let right = floating_restore_anchor(2300, 300, 280, 90);
    assert_eq!(floating_restore_bounds(right, taskbar, resolve), right_screen);
    // If the saved screen is unplugged or the settings had no coordinates,
    // use the known taskbar monitor rather than parking the UI off-screen.
    let unplugged = floating_restore_anchor(-4000, 0, 280, 90);
    assert_eq!(floating_restore_bounds(unplugged, taskbar, resolve), taskbar);
    assert!(floating_restore_anchor(i32::MIN, 100, 280, 90).is_none());
    assert!(floating_restore_anchor(100, i32::MIN, 280, 90).is_none());
    assert_eq!(floating_restore_bounds(None, taskbar, resolve), taskbar);

    // Taskbar and desktop position values remain separate across a
    // settings JSON round trip (including a docked edge).
    let saved = SettingsFile {
        taskbar_index: 0,
        taskbar_left_offset: Some(400),
        taskbar_monitor: Some("taskbar-monitor".into()),
        floating_x: Some(-1550),
        floating_y: Some(120),
        floating_edge: Some("left".into()),
        display_position: DisplayPosition::Desktop,
        ..SettingsFile::default()
    };
    let restored: SettingsFile = serde_json::from_str(
        &serde_json::to_string(&saved).unwrap()
    ).unwrap();
    assert_eq!(restored.taskbar_left_offset, Some(400));
    assert_eq!(restored.taskbar_monitor.as_deref(), Some("taskbar-monitor"));
    assert_eq!(restored.floating_x, Some(-1550));
    assert_eq!(restored.floating_y, Some(120));
    assert_eq!(restored.floating_edge.as_deref(), Some("left"));
    assert_eq!(restored.display_position, DisplayPosition::Desktop);
}

#[test]
fn desktop_horizontal_and_vertical_direction_round_trip() {
    let settings = SettingsFile {
        display_position: DisplayPosition::Desktop,
        desktop_direction: DesktopDirection::Vertical,
        ..Default::default()
    };
    let encoded = serde_json::to_string(&settings).unwrap();
    let parsed: SettingsFile = serde_json::from_str(&encoded).unwrap();
    assert_eq!(parsed.desktop_direction, DesktopDirection::Vertical);
    assert_eq!(parsed.display_position, DisplayPosition::Desktop);
    let legacy: SettingsFile = serde_json::from_str("{}").unwrap();
    assert_eq!(legacy.desktop_direction, DesktopDirection::Horizontal);
    assert_eq!(legacy.display_position, DisplayPosition::Taskbar);
}

#[test]
fn vertical_default_uses_compact_before_desktop_fallback() {
    let bar = RECT { left: 1800, top: 0, right: 1850, bottom: 800 };
    let ample = [VerticalSpan { top: 20, bottom: 280 }];
    assert_eq!(vertical_placement_for_gaps(AppearancePreset::Default, bar, &ample, 24),
        Some((AppearancePreset::Default, 24)));
    let short = [VerticalSpan { top: 20, bottom: 130 }];
    assert_eq!(vertical_placement_for_gaps(AppearancePreset::Default, bar, &short, 24),
        Some((AppearancePreset::Minimal, 24)));
    assert_eq!(vertical_placement_for_gaps(AppearancePreset::Default, bar, &short, 24),
        Some((AppearancePreset::Minimal, 24)));
    let exhausted = [VerticalSpan { top: 20, bottom: 65 }];
    assert_eq!(vertical_placement_for_gaps(AppearancePreset::Default, bar, &exhausted, 24), None);
    assert_eq!(vertical_placement_for_gaps(AppearancePreset::Minimal, bar, &short, 24),
        Some((AppearancePreset::Minimal, 24)));
}

#[test]
fn floating_widget_snaps_to_edges_and_preserves_a_visible_peek() {
    let bounds = RECT { left: -1920, top: -80, right: 0, bottom: 1000 };
    let (x, y) = floating_clamp_position(bounds, 48, 168, -3000, 1200);
    assert_eq!((x, y), (-1920, 832));
    assert_eq!(floating_nearest_edge(bounds,
        RECT { left: -1910, top: 200, right: -1862, bottom: 368 }, 24),
        Some(FloatingEdge::Left));
    assert_eq!(floating_nearest_edge(bounds,
        RECT { left: -1100, top: 200, right: -1052, bottom: 368 }, 24), None);
    assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
        Some(FloatingEdge::Left), true, 15),
        (bounds.left - floating_grip_rect(48, 168, Some(FloatingEdge::Left), 15).left, 100));
    assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
        Some(FloatingEdge::Left), false, 15), (-1920, 100));
    assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
        Some(FloatingEdge::Right), true, 15),
        (bounds.right - floating_grip_rect(48, 168, Some(FloatingEdge::Right), 15).right, 100));
    assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
        Some(FloatingEdge::Top), true, 15),
        (-1500, bounds.top - floating_grip_rect(48, 168, Some(FloatingEdge::Top), 15).top));
    assert_eq!(floating_edge_position(bounds, -1500, 100, 48, 168,
        Some(FloatingEdge::Bottom), true, 15),
        (-1500, bounds.bottom - floating_grip_rect(48, 168, Some(FloatingEdge::Bottom), 15).bottom));
    assert_eq!(FloatingEdge::parse("right"), Some(FloatingEdge::Right));
    assert_eq!(FloatingEdge::parse("unknown"), None);
}

#[test]
fn vertical_minimal_layout_shows_two_separate_remaining_percentages() {
    let height = vertical_widget_height_for_preset(AppearancePreset::Minimal);
    assert_eq!(height, sc(VERTICAL_MINIMAL_WIDGET_HEIGHT));
    assert_eq!(vertical_widget_height_for_preset(AppearancePreset::Default), sc(VERTICAL_WIDGET_HEIGHT));
    let (session, weekly) = vertical_minimal_percent_centers(height, true, true);
    let session = session.unwrap();
    let weekly = weekly.unwrap();
    assert!(session - sc(12) >= sc(VERTICAL_DRAG_HANDLE_H));
    assert!(weekly - session >= sc(24));
    assert!(weekly + sc(12) < height);
    assert_eq!(vertical_minimal_percent_centers(height, true, false).1, None);
    assert_eq!(vertical_minimal_percent_centers(height, false, true).0, None);
    // Stored percentages are USED; display is always REMAINING.
    assert_eq!(usage_percent_for_display(LanguageId::English, 88.0), 12.0);
    assert_eq!(usage_percent_for_display(LanguageId::English, 63.0), 37.0);
    assert_eq!(vertical_quota_text(12.0, true), "12%");
    assert_eq!(vertical_quota_text(37.0, true), "37%");
}

#[test]
fn vertical_default_tooltip_hits_only_reset_timestamp() {
    let width = sc(48);
    let height = sc(VERTICAL_WIDGET_HEIGHT);
    let (session, weekly) = vertical_row_tops(height, true, true);
    let first = session.unwrap();
    let second = weekly.unwrap();
    // Label, ring, center percentage and empty space do not trigger it.
    for y in [first + sc(VERTICAL_LABEL_CENTER_Y),
              first + sc(VERTICAL_RING_CENTER_Y), second + sc(VERTICAL_RING_CENTER_Y)] {
        assert!(vertical_usage_hover_hit(width, height, false, true, true, width / 2, y).is_none());
    }
    assert_eq!(vertical_usage_hover_hit(
        width, height, false, true, true,
        width / 2, first + sc(VERTICAL_RESET_CENTER_Y),
    ).unwrap().0, MinimalHoverTarget::Session);
    assert_eq!(vertical_usage_hover_hit(
        width, height, false, true, true,
        width / 2, second + sc(VERTICAL_RESET_CENTER_Y),
    ).unwrap().0, MinimalHoverTarget::Weekly);
    assert!(vertical_usage_hover_hit(width, height, false, true, true, sc(1),
        first + sc(VERTICAL_RESET_CENTER_Y)).is_none());
}

#[test]
fn vertical_minimal_tooltip_hits_percentages_not_drag_grip() {
    let width = sc(48);
    let height = sc(VERTICAL_MINIMAL_WIDGET_HEIGHT);
    let (session, weekly) = vertical_minimal_percent_centers(height, true, true);
    assert!(vertical_usage_hover_hit(width, height, true, true, true, width / 2, sc(8)).is_none());
    assert_eq!(vertical_usage_hover_hit(width, height, true, true, true,
        width / 2, session.unwrap()).unwrap().0, MinimalHoverTarget::Session);
    assert_eq!(vertical_usage_hover_hit(width, height, true, true, true,
        width / 2, weekly.unwrap()).unwrap().0, MinimalHoverTarget::Weekly);
    assert!(vertical_usage_hover_hit(width, height, true, true, true,
        sc(1), session.unwrap()).is_none());
}

#[test]
fn vertical_drag_tracks_y_outside_taskbar_without_leaving_parent() {
    let taskbar = RECT { left: 1900, top: -200, right: 1950, bottom: 800 };
    let anchor = 12;
    let height = 104;
    // Keep ordinary tracking when cursor is inside the vertical taskbar.
    assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1920, y: 240 }, anchor, height), 428);
    // Outside left/right keeps using Y and clamps to the current taskbar.
    assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1800, y: 240 }, anchor, height), 428);
    assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1990, y: -500 }, anchor, height), 0);
    assert_eq!(drag_top_for_pointer(taskbar, POINT { x: 1990, y: 1000 }, anchor, height), 896);
}

#[test]
fn taskbar_offsets_round_trip() {
    let old = r#"{"taskbar_index":0,"taskbar_left_offset":123}"#;
    let parsed: SettingsFile = serde_json::from_str(old).unwrap();
    assert_eq!(parsed.taskbar_left_offset, Some(123));
    assert_eq!(parsed.taskbar_top_offset, None);
    let mut new = parsed;
    new.taskbar_top_offset = Some(415);
    let serialized = serde_json::to_string(&new).unwrap();
    let decoded: SettingsFile = serde_json::from_str(&serialized).unwrap();
    assert_eq!(decoded.taskbar_top_offset, Some(415));
    assert_eq!(decoded.taskbar_left_offset, Some(123));
}

#[test]
fn free_taskbar_spans_exclude_occupied_controls_and_tray() {
    let taskbar = RECT {
        left: 0,
        top: 100,
        right: 1000,
        bottom: 148,
    };
    let occupied = [
        RECT {
            left: 400,
            top: 104,
            right: 520,
            bottom: 144,
        },
        RECT {
            left: 560,
            top: 104,
            right: 680,
            bottom: 144,
        },
    ];
    let free = free_spans_from_occupied(taskbar, 900, &occupied, 4);
    assert_eq!(
        free,
        vec![
            HorizontalSpan { left: 0, right: 396 },
            HorizontalSpan { left: 524, right: 556 },
            HorizontalSpan { left: 684, right: 900 },
        ]
    );
}

#[test]
fn free_space_snaps_to_nearest_safe_span() {
    let taskbar = RECT {
        left: 100,
        top: 0,
        right: 1100,
        bottom: 48,
    };
    let free = [
        HorizontalSpan { left: 100, right: 360 },
        HorizontalSpan { left: 700, right: 1000 },
    ];
    assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 650), Some(650));
    assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 900), Some(720));
    assert_eq!(nearest_free_left_offset(taskbar, &free, 180, 40), Some(40));
}

#[test]
fn default_layout_prefers_full_then_falls_back_to_minimal() {
    let taskbar = RECT {
        left: 0,
        top: 0,
        right: 1200,
        bottom: 48,
    };
    let default_width =
        total_widget_width_for_preset(LanguageId::English, AppearancePreset::Default);
    let minimal_width =
        total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);

    let wide = [HorizontalSpan {
        left: 100,
        right: 100 + default_width + 20,
    }];
    let placement = placement_for_free_spans(
        AppearancePreset::Default,
        LanguageId::English,
        taskbar,
        &wide,
        100,
    )
    .unwrap();
    assert_eq!(placement.0, AppearancePreset::Default);

    let narrow = [HorizontalSpan {
        left: 100,
        right: 100 + minimal_width,
    }];
    let placement = placement_for_free_spans(
        AppearancePreset::Default,
        LanguageId::English,
        taskbar,
        &narrow,
        100,
    )
    .unwrap();
    assert_eq!(placement.0, AppearancePreset::Minimal);
}

#[test]
fn default_layout_prioritizes_full_display_before_compact_elsewhere() {
    let taskbar = RECT {
        left: 0,
        top: 0,
        right: 1600,
        bottom: 48,
    };
    let default_width =
        total_widget_width_for_preset(LanguageId::English, AppearancePreset::Default);
    let minimal_width =
        total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);
    let free = [
        HorizontalSpan {
            left: 100,
            right: 100 + default_width + 20,
        },
        HorizontalSpan {
            left: 900,
            right: 900 + minimal_width,
        },
    ];
    let placement = placement_for_free_spans(
        AppearancePreset::Default,
        LanguageId::English,
        taskbar,
        &free,
        900,
    )
    .unwrap();
    assert_eq!(placement.0, AppearancePreset::Default);
    assert_eq!(placement.2, 120);
}

#[test]
fn default_layout_reports_no_placement_when_even_minimal_does_not_fit() {
    let taskbar = RECT {
        left: 0,
        top: 0,
        right: 1200,
        bottom: 48,
    };
    let minimal_width =
        total_widget_width_for_preset(LanguageId::English, AppearancePreset::Minimal);
    let too_small = [HorizontalSpan {
        left: 100,
        right: 100 + minimal_width - 1,
    }];
    assert!(placement_for_free_spans(
        AppearancePreset::Default,
        LanguageId::English,
        taskbar,
        &too_small,
        100,
    )
    .is_none());
}

#[test]
fn centers_widget_vertically() {
    assert_eq!(compute_anchor_y(100, 48, 42), 103);
    assert_eq!(compute_anchor_y(100, 32, 28), 102);
    assert_eq!(compute_anchor_y(100, 24, 28), 100);
}

#[test]
fn small_taskbar_threshold_is_dpi_aware() {
    assert!(is_small_taskbar_height_at_dpi(32, 96));
    assert!(is_small_taskbar_height_at_dpi(34, 96));
    assert!(!is_small_taskbar_height_at_dpi(35, 96));
    assert!(is_small_taskbar_height_at_dpi(51, 144));
    assert!(!is_small_taskbar_height_at_dpi(52, 144));
}

#[test]
fn tooltip_width_tracks_drawn_text_without_excess_reserve() {
    assert_eq!(tooltip_width_with_render_slack(100, |v| v), 127);
    assert_eq!(tooltip_width_with_render_slack(100, |v| v * 2), 154);
    assert_eq!(tooltip_width_with_render_slack(0, |v| v), 48);
    assert_eq!(tooltip_width_with_render_slack(0, |v| v * 2), 96);
    assert_eq!(tooltip_width_with_render_slack(124, |v| v), 151);
}

#[test]
fn tooltip_mask_measurement_preserves_rightmost_antialiased_pixel() {
    // Three rows, ten columns; the last visible digit reaches column 7.
    let mut mask = vec![0; 30];
    mask[12] = 255;
    mask[17] = 1;
    mask[24] = 200;
    assert_eq!(tooltip_mask_content_width(&mask, 10), Some(8));
    assert_eq!(tooltip_mask_content_width(&mask, 0), None);
    assert_eq!(tooltip_mask_content_width(&mask[..29], 10), None);
    assert_eq!(tooltip_mask_content_width(&[0; 10], 10), None);
    // Rendering the same content at higher DPI scales the width normally.
    assert_eq!(tooltip_width_with_render_slack(160, |v| v * 2), 214);
}

#[test]
fn service_tooltip_combines_visible_quota_rows() {
    assert_eq!(
        service_tooltip(
            "Codex",
            "剩余13% 19:04重置",
            "剩余86% 07/18重置",
            true,
            true,
        ),
        "Codex: 5H 剩余13% 19:04重置 | 7D 剩余86% 07/18重置"
    );
}

#[test]
fn empty_settings_default_to_default_appearance() {
    let settings: SettingsFile = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.appearance_preset, AppearancePreset::Default);
}

#[test]
fn display_position_and_retired_adaptive_value_is_rejected() {
    let settings: SettingsFile = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.display_position, DisplayPosition::Taskbar);
    let desktop = SettingsFile {
        display_position: DisplayPosition::Desktop,
        appearance_preset: AppearancePreset::Default,
        ..Default::default()
    };
    let parsed: SettingsFile = serde_json::from_str(
        &serde_json::to_string(&desktop).unwrap()
    ).unwrap();
    assert_eq!(parsed.display_position, DisplayPosition::Desktop);
    assert_eq!(parsed.appearance_preset, AppearancePreset::Default);
    assert!(serde_json::from_str::<AppearancePreset>(r#""adaptive""#).is_err());
}

#[test]
fn default_adapts_on_vertical_taskbar_without_mutating_selection() {
    let bar = RECT { left: 1800, top: 0, right: 1850, bottom: 800 };
    let ample = [VerticalSpan { top: 20, bottom: 280 }];
    let narrow = [VerticalSpan { top: 20, bottom: 130 }];
    let exhausted = [VerticalSpan { top: 20, bottom: 65 }];
    let configured = AppearancePreset::Default;
    assert_eq!(vertical_placement_for_gaps(configured, bar, &ample, 24).unwrap().0,
        AppearancePreset::Default);
    assert_eq!(vertical_placement_for_gaps(configured, bar, &narrow, 24).unwrap().0,
        AppearancePreset::Minimal);
    assert_eq!(vertical_placement_for_gaps(configured, bar, &exhausted, 24), None);
    assert_eq!(configured, AppearancePreset::Default);
}

#[test]
fn stable_taskbar_position_round_trips() {
    let settings = SettingsFile {
        taskbar_index: 1,
        taskbar_left_offset: Some(0),
        taskbar_monitor: Some(r"\\.\DISPLAY2".to_string()),
        ..Default::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    let parsed: SettingsFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.taskbar_left_offset, Some(0));
    assert_eq!(parsed.taskbar_monitor.as_deref(), Some(r"\\.\DISPLAY2"));
}

#[test]
fn drag_outside_taskbar_follows_pointer_x_with_fixed_taskbar_bounds() {
    let taskbar = RECT {
        left: 500, top: 1000, right: 1500, bottom: 1048,
    };
    let width = 200;
    let anchor = 30;
    // Y is above the taskbar, but horizontal movement still follows.
    assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 700, y: 900 }, anchor, width), 170);
    assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 900, y: 900 }, anchor, width), 370);
    // The full widget stays inside the taskbar when X exceeds its edges.
    assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 100, y: 900 }, anchor, width), 0);
    assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 1800, y: 900 }, anchor, width), 800);
    // Existing in-taskbar cursor positioning is unchanged.
    assert_eq!(drag_left_for_pointer(taskbar, POINT { x: 900, y: 1020 }, anchor, width), 370);
}

#[test]
fn drag_outside_taskbar_handles_negative_screen_coordinates() {
    let taskbar = RECT {
        left: -1600, top: 500, right: -600, bottom: 548,
    };
    assert_eq!(drag_left_for_pointer(
        taskbar, POINT { x: -1000, y: 300 }, 40, 200,
    ), 560);
    assert_eq!(drag_left_for_pointer(
        taskbar, POINT { x: -2000, y: 300 }, 40, 200,
    ), 0);
}

#[test]
fn transient_taskbar_clamp_does_not_change_desired_left_offset() {
    let desired = 0;
    assert_eq!(clamp_left_offset(desired, 800), 0);
    assert_eq!(clamp_left_offset(desired, 120), 0);

    let desired = 700;
    assert_eq!(clamp_left_offset(desired, 650), 650);
    assert_eq!(clamp_left_offset(desired, 800), 700);
}

#[test]
fn monitor_identity_wins_over_selected_taskbar_index() {
    let taskbars = vec![
        native_interop::TaskbarWindow {
            hwnd: HWND::default(),
            rect: RECT::default(),
            monitor_device: Some(r"\\.\DISPLAY1".to_string()),
        },
        native_interop::TaskbarWindow {
            hwnd: HWND::default(),
            rect: RECT::default(),
            monitor_device: Some(r"\\.\DISPLAY2".to_string()),
        },
    ];
    let (index, taskbar) =
        select_preferred_taskbar(&taskbars, Some(r"\\.\DISPLAY2"), 0).unwrap();
    assert_eq!(index, 1);
    assert_eq!(taskbar.monitor_device.as_deref(), Some(r"\\.\DISPLAY2"));
}

#[test]
fn explicit_minimal_appearance_round_trips() {
    let settings = SettingsFile {
        appearance_preset: AppearancePreset::Minimal,
        ..Default::default()
    };
    let json = serde_json::to_string(&settings).unwrap();
    let parsed: SettingsFile = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.appearance_preset, AppearancePreset::Minimal);
}

#[test]
fn schema_repair_retains_matching_fields_and_removes_nested_obsolete_names() {
    let mut config = serde_json::to_value(SettingsFile::default()).unwrap();
    config["appearance_preset"] = serde_json::json!("adaptive");
    config["display_position"] = serde_json::json!("desktop");
    config["desktop_direction"] = serde_json::json!("vertical");
    config["taskbar_left_offset"] = serde_json::json!(163);
    config["styles"]["dark"]["remaining"] = serde_json::json!("#ABCDEF88");
    config["styles"]["dark"]["removed_field"] = serde_json::json!(123);
    config["obsolete_setting"] = serde_json::json!("ignored");
    config.as_object_mut().unwrap().remove("poll_interval_ms");
    let (settings, dirty) = reconcile_settings_json(&config.to_string());
    assert!(dirty);
    assert_eq!(settings.appearance_preset, AppearancePreset::Default);
    assert_eq!(settings.display_position, DisplayPosition::Desktop);
    assert_eq!(settings.desktop_direction, DesktopDirection::Vertical);
    assert_eq!(settings.taskbar_left_offset, Some(163));
    assert_eq!(settings.poll_interval_ms, POLL_15_MIN);
    assert_eq!(settings.styles.dark.remaining, "#ABCDEF88");
    let saved = serde_json::to_value(&settings).unwrap();
    assert!(saved.get("obsolete_setting").is_none());
    assert!(saved["styles"]["dark"].get("removed_field").is_none());
    assert!(!reconcile_settings_json(&saved.to_string()).1);
}

#[test]
fn schema_repair_defaults_bad_leaf_without_losing_other_user_values() {
    let bad = serde_json::json!({
        "appearance_preset": "old-variant",
        "theme_mode": "dark",
        "poll_interval_ms": "wrong-type",
        "taskbar_left_offset": 280,
        "styles": { "dark": {
            "panel_corner_radius": "invalid",
            "remaining": "#123456FF"
        }}
    });
    let (settings, changed) = reconcile_settings_json(&bad.to_string());
    assert!(changed);
    assert_eq!(settings.appearance_preset, AppearancePreset::Default);
    assert_eq!(settings.theme_mode, ThemeMode::Dark);
    assert_eq!(settings.poll_interval_ms, POLL_15_MIN);
    assert_eq!(settings.taskbar_left_offset, Some(280));
    assert_eq!(settings.styles.dark.remaining, "#123456FF");
    assert_eq!(settings.styles.dark.panel_corner_radius,
        ThemeStyle::dark_default().panel_corner_radius);
}

#[test]
fn unsupported_semantic_values_are_restored_to_current_defaults() {
    let invalid = serde_json::json!({
        "poll_interval_ms": 0,
        "floating_edge": "diagonal",
        "language": "zh-CN",
        "display_position": "desktop"
    });
    let (settings, changed) = reconcile_settings_json(&invalid.to_string());
    assert!(changed);
    assert_eq!(settings.poll_interval_ms, POLL_15_MIN);
    assert_eq!(settings.floating_edge, None);
    assert_eq!(settings.language.as_deref(), Some("zh-CN"));
    assert_eq!(settings.display_position, DisplayPosition::Desktop);
}

#[test]
fn current_schema_discards_retired_language_and_old_shape() {
    let input = serde_json::json!({
        "language": "ja-JP",
        "styles": {"dark": {"panel_rounded": true}},
        "tray_offset": 123,
        "taskbar_left_offset": 80,
    });
    let (settings, changed) = reconcile_settings_json(&input.to_string());
    assert!(changed);
    assert_eq!(settings.language, None);
    assert_eq!(settings.taskbar_left_offset, Some(80));
    assert_eq!(settings.styles.dark.panel_corner_radius, ThemeStyle::dark_default().panel_corner_radius);
    let output = serde_json::to_value(settings).unwrap();
    assert!(output.get("tray_offset").is_none());
    assert!(output["styles"]["dark"].get("panel_rounded").is_none());
}

#[test]
fn malformed_settings_are_backed_up_before_recovery() {
    let dir = std::env::temp_dir().join(format!(
        "codex-settings-invalid-{}-{}", std::process::id(), now_unix_secs()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let current = dir.join("settings.json");
    std::fs::write(&current, "{invalid json").unwrap();
    let (settings, rewrite) = load_settings_from_path(&current).unwrap();
    assert!(rewrite);
    assert_eq!(settings.appearance_preset, AppearancePreset::Default);
    assert_eq!(std::fs::read_to_string(current.with_extension("json.invalid.bak")).unwrap(), "{invalid json");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn displays_distinct_transient_error_categories() {
    assert_eq!(
        poll_error_display_label(
            poller::PollError::NetworkUnavailable,
            LanguageId::SimplifiedChinese
        ),
        "网络"
    );
    assert_eq!(
        poll_error_display_label(
            poller::PollError::RateLimited,
            LanguageId::SimplifiedChinese
        ),
        "限流"
    );
    assert_eq!(
        poll_error_display_label(poller::PollError::ServerError, LanguageId::English),
        "5XX"
    );
    assert_eq!(
        poll_error_display_label(poller::PollError::RequestFailed, LanguageId::English),
        "ERR"
    );
}

#[test]
fn normalizes_usage_display_and_alert_settings() {
    let settings = normalize_settings(SettingsFile {
        show_session_window: false,
        show_weekly_window: false,
        alert_threshold_percent: 17,
        notified_quota_windows: vec!["codex:weekly:1".into(), "codex:weekly:1".into()],
        ..SettingsFile::default()
    });
    assert!(settings.show_session_window);
    assert!(!settings.show_weekly_window);
    assert_eq!(settings.alert_threshold_percent, 0);
    assert_eq!(settings.notified_quota_windows.len(), 1);
}

#[test]
fn formats_precise_local_reset_time() {
    let local = SYSTEMTIME {
        wYear: 2026,
        wMonth: 7,
        wDay: 17,
        wHour: 18,
        wMinute: 30,
        ..Default::default()
    };
    assert_eq!(format_local_system_time(local), "2026-07-17 18:30");
    assert_eq!(format_precise_reset_time(None), None);
}

#[test]
fn low_quota_alert_is_deduplicated_until_reset_window_changes() {
    let mut alerts = Vec::new();
    let mut notified = BTreeSet::new();
    let first_reset = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
    let first = crate::models::UsageSection {
        percentage: 85.0,
        resets_at: Some(first_reset),
    };
    append_quota_alert(
        &mut alerts,
        &mut notified,
        20,
        LanguageId::SimplifiedChinese,
        tray_icon::TrayIconKind::Codex,
        "codex",
        "Codex",
        "session",
        "5小时",
        &first,
    );
    append_quota_alert(
        &mut alerts,
        &mut notified,
        20,
        LanguageId::SimplifiedChinese,
        tray_icon::TrayIconKind::Codex,
        "codex",
        "Codex",
        "session",
        "5小时",
        &first,
    );
    assert_eq!(alerts.len(), 1);
    assert!(alerts[0].message.contains("剩余 15%"));

    let next = crate::models::UsageSection {
        percentage: 90.0,
        resets_at: Some(first_reset + Duration::from_secs(18_000)),
    };
    append_quota_alert(
        &mut alerts,
        &mut notified,
        20,
        LanguageId::SimplifiedChinese,
        tray_icon::TrayIconKind::Codex,
        "codex",
        "Codex",
        "session",
        "5小时",
        &next,
    );
    assert_eq!(alerts.len(), 2);
    assert_eq!(notified.len(), 1);
}

