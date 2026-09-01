//! Platform and presentation configuration: app identity per build
//! channel, the shell window/view scene, permissions, tray items, and
//! the Precision design-token theme.

const std = @import("std");
const codecaddie_build = @import("codecaddie_build");
const native_sdk = @import("native_sdk");

const model_mod = @import("model.zig");

const canvas = native_sdk.canvas;
const Model = model_mod.Model;

pub const canvas_label = "codecaddie-canvas";
pub const window_width: f32 = 1440;
pub const window_height: f32 = 1024;
const development_channel = std.mem.eql(u8, codecaddie_build.channel, "dev");
pub const app_display_name = if (development_channel) "CodeCaddie Dev" else "CodeCaddie";
pub const app_bundle_id = if (development_channel) "org.codecaddie.desktop.dev" else "org.codecaddie.desktop";

pub const app_permissions = [_][]const u8{
    native_sdk.security.permission_command,
    native_sdk.security.permission_view,
};

const shell_views = [_]native_sdk.ShellView{.{
    .label = canvas_label,
    .kind = .gpu_surface,
    .fill = true,
    .role = "CodeCaddie project analysis",
    .accessibility_label = "CodeCaddie",
    .gpu_backend = .metal,
    .gpu_pixel_format = .bgra8_unorm,
    .gpu_present_mode = .timer,
    .gpu_alpha_mode = .@"opaque",
    .gpu_color_space = .srgb,
    .gpu_vsync = true,
}};

const shell_windows = [_]native_sdk.ShellWindow{.{
    .label = "main",
    .title = app_display_name,
    .width = window_width,
    .height = window_height,
    .min_width = 960,
    .min_height = 700,
    .restore_state = true,
    .views = &shell_views,
}};

pub const shell_scene: native_sdk.ShellConfig = .{ .windows = &shell_windows };

pub const tray_items = [_]native_sdk.platform.TrayMenuItem{.{ .id = 1, .label = "Show CodeCaddie", .command = "app.show" }};

// Precision theme. Typography stays on the SDK's built-in faces so
// medium/bold spans map to real weight faces on every host, and high
// contrast is no longer a typeface swap. Accent is the one electric hue,
// reserved for live/latest signals; primary controls stay ink-on-paper.
// docs/BRAND.md mirrors this table and must not drift from it.
pub fn tokens(model: *const Model) canvas.DesignTokens {
    var result = canvas.DesignTokens.theme(.{ .pack = .geist, .color_scheme = if (model.dark) .dark else .light, .contrast = if (model.high_contrast) .high else .standard, .reduce_motion = model.reduce_motion });
    if (model.high_contrast) return result;
    result.typography.heading_size = 20;
    result.typography.display_size = 32;
    result.radius.sm = 4;
    result.radius.md = 6;
    result.radius.lg = 8;
    result.radius.xl = 12;
    if (model.dark) {
        result.colors.background = canvas.Color.rgb8(10, 10, 10);
        result.colors.surface = canvas.Color.rgb8(17, 17, 17);
        result.colors.surface_subtle = canvas.Color.rgb8(25, 25, 25);
        result.colors.surface_pressed = canvas.Color.rgb8(35, 35, 35);
        result.colors.text = canvas.Color.rgb8(237, 237, 237);
        result.colors.text_muted = canvas.Color.rgb8(161, 161, 161);
        result.colors.border = canvas.Color.rgb8(44, 44, 44);
        result.colors.accent = canvas.Color.rgb8(82, 169, 255);
        result.colors.accent_text = canvas.Color.rgb8(8, 36, 63);
        result.colors.info = canvas.Color.rgb8(82, 169, 255);
        result.colors.info_text = canvas.Color.rgb8(8, 36, 63);
        result.colors.focus_ring = canvas.Color.rgb8(84, 194, 255);
        result.colors.success = canvas.Color.rgb8(86, 194, 113);
        result.colors.success_text = canvas.Color.rgb8(8, 36, 15);
        result.colors.warning = canvas.Color.rgb8(237, 180, 49);
        result.colors.warning_text = canvas.Color.rgb8(36, 26, 2);
        result.colors.destructive = canvas.Color.rgb8(244, 124, 124);
        result.colors.destructive_text = canvas.Color.rgb8(43, 10, 10);
        result.colors.disabled = canvas.Color.rgb8(92, 92, 92);
        result.colors.shadow = canvas.Color.rgba8(0, 0, 0, 102);
        result.colors.scrim = canvas.Color.rgba8(0, 0, 0, 166);
        // The syntax tokens serve double duty: snippet foregrounds AND the
        // heatmap status hues (keyword=Broken, constant=Incomplete,
        // literal=Functional). Every value must hold >= 4.5:1 as text on
        // surface_subtle and >= 3:1 as a status fill against surface.
        result.colors.syntax_keyword = canvas.Color.rgb8(224, 136, 90);
        result.colors.syntax_literal = canvas.Color.rgb8(79, 195, 217);
        result.colors.syntax_constant = canvas.Color.rgb8(210, 166, 56);
        result.colors.syntax_plain = canvas.Color.rgb8(237, 237, 237);
        result.colors.syntax_comment = canvas.Color.rgb8(154, 154, 154);
        result.colors.syntax_function = canvas.Color.rgb8(121, 189, 255);
        result.colors.syntax_property = canvas.Color.rgb8(124, 217, 154);
    } else {
        result.colors.background = canvas.Color.rgb8(250, 250, 250);
        result.colors.surface = canvas.Color.rgb8(255, 255, 255);
        result.colors.surface_subtle = canvas.Color.rgb8(244, 244, 244);
        result.colors.surface_pressed = canvas.Color.rgb8(232, 232, 232);
        result.colors.text = canvas.Color.rgb8(23, 23, 23);
        result.colors.text_muted = canvas.Color.rgb8(102, 102, 102);
        result.colors.border = canvas.Color.rgb8(228, 228, 228);
        result.colors.accent = canvas.Color.rgb8(0, 98, 214);
        result.colors.accent_text = canvas.Color.rgb8(255, 255, 255);
        result.colors.info = canvas.Color.rgb8(0, 98, 214);
        result.colors.info_text = canvas.Color.rgb8(255, 255, 255);
        result.colors.focus_ring = canvas.Color.rgb8(0, 95, 204);
        result.colors.success = canvas.Color.rgb8(15, 123, 63);
        result.colors.success_text = canvas.Color.rgb8(255, 255, 255);
        result.colors.warning = canvas.Color.rgb8(154, 103, 0);
        result.colors.warning_text = canvas.Color.rgb8(255, 255, 255);
        result.colors.destructive = canvas.Color.rgb8(196, 43, 43);
        result.colors.destructive_text = canvas.Color.rgb8(255, 255, 255);
        result.colors.disabled = canvas.Color.rgb8(163, 163, 163);
        result.colors.shadow = canvas.Color.rgba8(0, 0, 0, 15);
        result.colors.scrim = canvas.Color.rgba8(0, 0, 0, 89);
        // Same double duty as the dark branch: legible snippet text that is
        // also the Broken/Incomplete/Functional status hue.
        result.colors.syntax_keyword = canvas.Color.rgb8(154, 59, 18);
        result.colors.syntax_literal = canvas.Color.rgb8(14, 116, 144);
        result.colors.syntax_constant = canvas.Color.rgb8(138, 100, 0);
        result.colors.syntax_plain = canvas.Color.rgb8(23, 23, 23);
        result.colors.syntax_comment = canvas.Color.rgb8(110, 110, 110);
        result.colors.syntax_function = canvas.Color.rgb8(0, 98, 214);
        result.colors.syntax_property = canvas.Color.rgb8(15, 123, 63);
    }
    const ink = if (model.dark) canvas.Color.rgb8(237, 237, 237) else canvas.Color.rgb8(23, 23, 23);
    const ink_text = if (model.dark) canvas.Color.rgb8(10, 10, 10) else canvas.Color.rgb8(255, 255, 255);
    const ink_hover = if (model.dark) canvas.Color.rgb8(200, 200, 200) else canvas.Color.rgb8(51, 51, 51);
    result.controls.button_primary.background = ink;
    result.controls.button_primary.foreground = ink_text;
    result.controls.button_primary.hover_background = ink_hover;
    result.controls.toggle_button.active_background = ink;
    result.controls.toggle_button.active_foreground = ink_text;
    // Destructive buttons keep >= 4.5:1 label contrast in every state
    // (the SDK pack's dark hover step falls to 3.98:1 under white text).
    result.controls.button_destructive.background = result.colors.destructive;
    result.controls.button_destructive.foreground = result.colors.destructive_text;
    result.controls.button_destructive.hover_background = if (model.dark) canvas.Color.rgb8(247, 146, 146) else canvas.Color.rgb8(163, 33, 33);
    // Field boundaries hold >= 3:1 against the page, surface, and pressed
    // fills so low-vision users can find field extents before focus; the
    // decorative card hairline stays on the quiet `colors.border` step.
    const field_border = if (model.dark) canvas.Color.rgb8(110, 110, 110) else canvas.Color.rgb8(134, 134, 134);
    result.controls.text_field.border = field_border;
    result.controls.textarea.border = field_border;
    result.controls.input.border = field_border;
    result.controls.search_field.border = field_border;
    result.controls.select.border = field_border;
    result.controls.combobox.border = field_border;
    return result;
}
