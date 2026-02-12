use js_sys::{Array, ArrayBuffer, Function, Object, Promise, Reflect, Uint8Array, WebAssembly};
use qrcodegen::{QrCode, QrCodeEcc};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    CanvasRenderingContext2d, Document, Event, HtmlButtonElement, HtmlCanvasElement,
    HtmlDivElement, HtmlElement, HtmlImageElement, PointerEvent, Response, WebGlBuffer,
    WebGlProgram,
    WebGlRenderingContext as Gl, WebGlShader, WebGlTexture, WebGlUniformLocation, Window,
};

const VERTEX_SHADER_SOURCE: &str = r#"
attribute vec2 a_position;
attribute vec2 a_texCoord;
varying vec2 v_texCoord;
void main() {
  gl_Position = vec4(a_position, 0.0, 1.0);
  v_texCoord = a_texCoord;
}
"#;

const FRAGMENT_SHADER_SOURCE: &str = r#"
precision mediump float;
varying vec2 v_texCoord;
uniform sampler2D u_texture;
void main() {
  gl_FragColor = texture2D(u_texture, v_texCoord);
}
"#;

const LEVEL_WASM_URL: &str = "assets/levels/mall_parking_lot.wasm?v=unit11";
const SW_BOOTSTRAP_URL: &str = "/sw_bootstrap_sync.js?v=sync-init-13";
const ALIEN_SPRITE_BMP: &[u8] = include_bytes!("../assets/characters/alien_256.bmp");
const DEMON_SPRITE_BMP: &[u8] = include_bytes!("../assets/characters/demon_256.bmp");
const REF_LEVEL_WIDTH: f64 = 288.0;
const REF_LEVEL_HEIGHT: f64 = 512.0;
const BASE_CY_FROM_EDGE_REF: f64 = 66.0;
const BASE_RADIUS_REF: f64 = 30.0;
const SPAWN_SPRITE_H_REF: f64 = 44.0;
const SPAWN_SPRITE_SELECT_SCALE: f64 = 1.16;
const REF_LANE_CENTERS: [f64; 3] = [86.0, 144.0, 202.0];
const REF_LANE_WIDTH: f64 = 36.0;
const REF_LANE_TOP: f64 = 108.0;
const REF_LANE_BOTTOM: f64 = 404.0;

#[derive(Clone, Copy)]
struct DrawInfo {
    frame_width: f64,
    frame_height: f64,
    frame_x: f64,
    frame_y: f64,
    scale: f64,
    draw_width: f64,
    draw_height: f64,
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scene {
    Title,
    LoadingLevel,
    Level,
}

struct LevelMap {
    width: u32,
    height: u32,
    pixels_rgba: Vec<u8>,
}

#[derive(Clone)]
struct Sprite {
    width: u32,
    height: u32,
    pixels_rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectedSpawn {
    Alien,
    Demon,
}

enum LevelLoadState {
    NotStarted,
    Loading,
    Ready(LevelMap),
    Error(String),
}

struct AppState {
    gl: Gl,
    program: WebGlProgram,
    position_buffer: WebGlBuffer,
    tex_coord_buffer: WebGlBuffer,
    texture: WebGlTexture,
    a_position: u32,
    a_tex_coord: u32,
    u_texture: WebGlUniformLocation,
    canvas: HtmlCanvasElement,
    diagnostics: HtmlDivElement,
    diagnostics_text: HtmlElement,
    lan_share: HtmlDivElement,
    lan_qr: HtmlImageElement,
    lan_url: HtmlElement,
    tools_button: HtmlButtonElement,
    diagnostics_open: bool,
    fallback: Option<HtmlDivElement>,
    image: HtmlImageElement,
    title_loaded: bool,
    scene: Scene,
    content_width: u32,
    content_height: u32,
    level_state: LevelLoadState,
    selected_spawn: Option<SelectedSpawn>,
    alien_position: Option<(i32, i32)>,
    demon_position: Option<(i32, i32)>,
    alien_sprite: Option<Sprite>,
    demon_sprite: Option<Sprite>,
    context_lost: bool,
    draw_info: Option<DrawInfo>,
    hud_frame_css: Option<(i32, i32, i32, i32)>,
    document: Document,
    user_agent: String,
    gl_version: String,
    gl_renderer: String,
    gl_vendor: String,
    gl_max_texture_size: i32,
    gl_max_renderbuffer_size: i32,
    last_gl_error: Option<String>,
    last_event: String,
}

fn window() -> Window {
    web_sys::window().expect("missing window")
}

fn should_register_service_worker() -> Result<(), String> {
    let location = window().location();
    let search = location.search().unwrap_or_default();
    if search.contains("nosw=1") {
        return Err("disabled via nosw=1".to_string());
    }

    Ok(())
}

fn service_worker_container() -> Result<JsValue, String> {
    let navigator = window().navigator();
    let nav_js: JsValue = navigator.into();

    let has_sw = Reflect::has(&nav_js, &JsValue::from_str("serviceWorker"))
        .map_err(|err| js_value_to_string(&err))?;
    if !has_sw {
        return Err("service worker unsupported".to_string());
    }

    let sw_container = Reflect::get(&nav_js, &JsValue::from_str("serviceWorker"))
        .map_err(|err| js_value_to_string(&err))?;
    if sw_container.is_undefined() || sw_container.is_null() {
        return Err("service worker unavailable".to_string());
    }

    Ok(sw_container)
}

fn js_function(target: &JsValue, name: &str) -> Result<Function, String> {
    Reflect::get(target, &JsValue::from_str(name))
        .map_err(|err| js_value_to_string(&err))?
        .dyn_into::<Function>()
        .map_err(|_| format!("{} missing", name))
}

fn registration_script_url(registration: &JsValue) -> Option<String> {
    for key in ["active", "waiting", "installing"] {
        let Ok(worker) = Reflect::get(registration, &JsValue::from_str(key)) else {
            continue;
        };
        if worker.is_null() || worker.is_undefined() {
            continue;
        }
        let Ok(url) = Reflect::get(&worker, &JsValue::from_str("scriptURL")) else {
            continue;
        };
        if let Some(url) = url.as_string() {
            return Some(url);
        }
    }

    None
}

async fn unregister_registration(registration: &JsValue) -> Result<(), String> {
    let unregister = js_function(registration, "unregister")?;
    let promise = unregister
        .call0(registration)
        .map_err(|err| js_value_to_string(&err))?;
    let _ = JsFuture::from(Promise::from(promise))
        .await
        .map_err(|err| js_value_to_string(&err))?;
    Ok(())
}

async fn prune_stale_service_worker_registrations(sw_container: &JsValue) -> Result<u32, String> {
    let get_registrations = js_function(sw_container, "getRegistrations")?;
    let registrations_value = get_registrations
        .call0(sw_container)
        .map_err(|err| js_value_to_string(&err))?;
    let registrations_value = JsFuture::from(Promise::from(registrations_value))
        .await
        .map_err(|err| js_value_to_string(&err))?;
    let registrations = Array::from(&registrations_value);
    let mut removed = 0;

    for registration in registrations.iter() {
        let Some(url) = registration_script_url(&registration) else {
            continue;
        };
        if url.contains(SW_BOOTSTRAP_URL) {
            continue;
        }
        unregister_registration(&registration).await?;
        removed += 1;
    }

    Ok(removed)
}

async fn register_service_worker_async() -> Result<String, String> {
    should_register_service_worker()?;

    let sw_container = service_worker_container()?;
    let removed = prune_stale_service_worker_registrations(&sw_container).await?;
    let register = js_function(&sw_container, "register")
        .map_err(|_| "serviceWorker.register missing".to_string())?;

    let options = Object::new();
    Reflect::set(
        &options,
        &JsValue::from_str("type"),
        &JsValue::from_str("module"),
    )
    .map_err(|err| js_value_to_string(&err))?;

    let registration = register
        .call2(
            &sw_container,
            &JsValue::from_str(SW_BOOTSTRAP_URL),
            &JsValue::from(options),
        )
        .map_err(|err| js_value_to_string(&err))?;
    let registration = JsFuture::from(Promise::from(registration))
        .await
        .map_err(|err| js_value_to_string(&err))?;
    let script_url = registration_script_url(&registration).unwrap_or_else(|| "(pending)".into());

    Ok(format!(
        "sw_register (removed_stale={}, script={})",
        removed, script_url
    ))
}

fn compute_frame(canvas_width: f64, canvas_height: f64) -> (f64, f64, f64, f64) {
    let target_aspect = 9.0 / 16.0;
    let canvas_aspect = canvas_width / canvas_height;

    let (frame_width, frame_height) = if canvas_aspect > target_aspect {
        (canvas_height * target_aspect, canvas_height)
    } else {
        (canvas_width, canvas_width / target_aspect)
    };

    let frame_x = (canvas_width - frame_width) * 0.5;
    let frame_y = (canvas_height - frame_height) * 0.5;

    (frame_width, frame_height, frame_x, frame_y)
}

fn js_value_to_string(value: &JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{:?}", value))
}

fn gl_error_name(error: u32) -> &'static str {
    match error {
        Gl::NO_ERROR => "NO_ERROR",
        Gl::INVALID_ENUM => "INVALID_ENUM",
        Gl::INVALID_VALUE => "INVALID_VALUE",
        Gl::INVALID_OPERATION => "INVALID_OPERATION",
        Gl::OUT_OF_MEMORY => "OUT_OF_MEMORY",
        Gl::INVALID_FRAMEBUFFER_OPERATION => "INVALID_FRAMEBUFFER_OPERATION",
        Gl::CONTEXT_LOST_WEBGL => "CONTEXT_LOST_WEBGL",
        _ => "UNKNOWN_ERROR",
    }
}

fn gl_check(gl: &Gl, label: &str) -> Option<String> {
    let error = gl.get_error();
    if error == Gl::NO_ERROR {
        None
    } else {
        Some(format!(
            "gl error after {}: {} (0x{:x})",
            label,
            gl_error_name(error),
            error
        ))
    }
}

fn sw_control_status() -> (bool, bool, Option<String>) {
    let navigator = window().navigator();
    let nav_js: JsValue = navigator.into();
    let has_sw = Reflect::has(&nav_js, &JsValue::from_str("serviceWorker")).unwrap_or(false);
    if !has_sw {
        return (false, false, None);
    }

    let Ok(sw_container) = Reflect::get(&nav_js, &JsValue::from_str("serviceWorker")) else {
        return (true, false, None);
    };
    if sw_container.is_undefined() || sw_container.is_null() {
        return (true, false, None);
    }

    let controller = Reflect::get(&sw_container, &JsValue::from_str("controller"))
        .ok()
        .filter(|v| !v.is_null() && !v.is_undefined());

    let controller_url = controller.as_ref().and_then(|controller| {
        Reflect::get(controller, &JsValue::from_str("scriptURL"))
            .ok()
            .and_then(|v| v.as_string())
    });

    (true, controller.is_some(), controller_url)
}

fn scene_name(scene: Scene) -> &'static str {
    match scene {
        Scene::Title => "title",
        Scene::LoadingLevel => "loading_level",
        Scene::Level => "level",
    }
}

fn level_state_summary(state: &LevelLoadState) -> String {
    match state {
        LevelLoadState::NotStarted => "not_started".to_string(),
        LevelLoadState::Loading => "loading".to_string(),
        LevelLoadState::Ready(level) => {
            format!("ready ({}x{})", level.width, level.height)
        }
        LevelLoadState::Error(message) => format!("error ({})", message),
    }
}

fn selected_spawn_name(selected: Option<SelectedSpawn>) -> &'static str {
    match selected {
        Some(SelectedSpawn::Alien) => "alien",
        Some(SelectedSpawn::Demon) => "demon",
        None => "none",
    }
}

fn position_name(pos: Option<(i32, i32)>) -> String {
    match pos {
        Some((x, y)) => format!("{},{}", x, y),
        None => "(spawn)".to_string(),
    }
}

#[inline]
fn read_le_u16(bytes: &[u8], at: usize) -> Option<u16> {
    let end = at.checked_add(2)?;
    let b = bytes.get(at..end)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

#[inline]
fn read_le_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let end = at.checked_add(4)?;
    let b = bytes.get(at..end)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn read_le_i32(bytes: &[u8], at: usize) -> Option<i32> {
    let end = at.checked_add(4)?;
    let b = bytes.get(at..end)?;
    Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[inline]
fn bitfield_shift(mask: u32) -> u32 {
    mask.trailing_zeros()
}

#[inline]
fn bitfield_max(mask: u32) -> u32 {
    let shifted = mask >> bitfield_shift(mask);
    shifted.max(1)
}

#[inline]
fn extract_channel(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shifted = (v & mask) >> bitfield_shift(mask);
    ((shifted * 255) / bitfield_max(mask)) as u8
}

fn decode_bmp_to_rgba(bytes: &[u8]) -> Option<Sprite> {
    if bytes.get(0..2)? != b"BM" {
        return None;
    }

    let data_offset = read_le_u32(bytes, 10)? as usize;
    let dib_size = read_le_u32(bytes, 14)? as usize;
    if dib_size < 40 {
        return None;
    }

    let width_raw = read_le_i32(bytes, 18)?;
    let height_raw = read_le_i32(bytes, 22)?;
    let planes = read_le_u16(bytes, 26)?;
    let bpp = read_le_u16(bytes, 28)?;
    let compression = read_le_u32(bytes, 30)?;
    if planes != 1 {
        return None;
    }

    let width = width_raw.unsigned_abs();
    let height = height_raw.unsigned_abs();
    if width == 0 || height == 0 {
        return None;
    }
    let top_down = height_raw < 0;
    let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];

    match bpp {
        24 => {
            if compression != 0 {
                return None;
            }
            let row_stride = (width as usize * 3).div_ceil(4) * 4;
            let payload_len = row_stride.checked_mul(height as usize)?;
            let payload = bytes.get(data_offset..data_offset + payload_len)?;

            for y in 0..height as usize {
                let src_y = if top_down {
                    y
                } else {
                    height as usize - 1 - y
                };
                let row = &payload[(src_y * row_stride)..((src_y + 1) * row_stride)];
                for x in 0..width as usize {
                    let si = x * 3;
                    let di = (y * width as usize + x) * 4;
                    pixels[di] = row[si + 2];
                    pixels[di + 1] = row[si + 1];
                    pixels[di + 2] = row[si];
                    pixels[di + 3] = 255;
                }
            }
        }
        32 => {
            let row_stride = width as usize * 4;
            let payload_len = row_stride.checked_mul(height as usize)?;
            let payload = bytes.get(data_offset..data_offset + payload_len)?;

            let (rmask, gmask, bmask, amask) = if compression == 3 && dib_size >= 56 {
                (
                    read_le_u32(bytes, 54)?,
                    read_le_u32(bytes, 58)?,
                    read_le_u32(bytes, 62)?,
                    read_le_u32(bytes, 66)?,
                )
            } else {
                (0x00FF0000, 0x0000FF00, 0x000000FF, 0xFF000000)
            };

            for y in 0..height as usize {
                let src_y = if top_down {
                    y
                } else {
                    height as usize - 1 - y
                };
                let row = &payload[(src_y * row_stride)..((src_y + 1) * row_stride)];
                for x in 0..width as usize {
                    let si = x * 4;
                    let di = (y * width as usize + x) * 4;
                    let px = u32::from_le_bytes([row[si], row[si + 1], row[si + 2], row[si + 3]]);
                    pixels[di] = extract_channel(px, rmask);
                    pixels[di + 1] = extract_channel(px, gmask);
                    pixels[di + 2] = extract_channel(px, bmask);
                    pixels[di + 3] = extract_channel(px, amask);
                }
            }
        }
        _ => return None,
    }

    Some(Sprite {
        width,
        height,
        pixels_rgba: pixels,
    })
}

#[inline]
fn color_dist_sq(r: u8, g: u8, b: u8, bg: [u8; 3]) -> u32 {
    let dr = (r as i32) - (bg[0] as i32);
    let dg = (g as i32) - (bg[1] as i32);
    let db = (b as i32) - (bg[2] as i32);
    ((dr * dr) + (dg * dg) + (db * db)) as u32
}

fn key_background_transparent(sprite: &mut Sprite) {
    let w = sprite.width as usize;
    let h = sprite.height as usize;
    if w == 0 || h == 0 {
        return;
    }

    let mut border_samples: Vec<[u8; 3]> = Vec::new();
    let step_x = (w / 12).max(1);
    let step_y = (h / 12).max(1);

    for x in (0..w).step_by(step_x) {
        let top_i = x * 4;
        let bot_i = ((h - 1) * w + x) * 4;
        border_samples.push([
            sprite.pixels_rgba[top_i],
            sprite.pixels_rgba[top_i + 1],
            sprite.pixels_rgba[top_i + 2],
        ]);
        border_samples.push([
            sprite.pixels_rgba[bot_i],
            sprite.pixels_rgba[bot_i + 1],
            sprite.pixels_rgba[bot_i + 2],
        ]);
    }
    for y in (0..h).step_by(step_y) {
        let left_i = (y * w) * 4;
        let right_i = (y * w + (w - 1)) * 4;
        border_samples.push([
            sprite.pixels_rgba[left_i],
            sprite.pixels_rgba[left_i + 1],
            sprite.pixels_rgba[left_i + 2],
        ]);
        border_samples.push([
            sprite.pixels_rgba[right_i],
            sprite.pixels_rgba[right_i + 1],
            sprite.pixels_rgba[right_i + 2],
        ]);
    }

    if border_samples.is_empty() {
        return;
    }

    let mut sum = [0u32; 3];
    for s in &border_samples {
        sum[0] += s[0] as u32;
        sum[1] += s[1] as u32;
        sum[2] += s[2] as u32;
    }
    let n = border_samples.len() as u32;
    let bg = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8];

    let mut spread = 0u32;
    for s in &border_samples {
        spread = spread.max(color_dist_sq(s[0], s[1], s[2], bg));
    }
    let threshold = (spread + 900).clamp(900, 6400);

    for px in sprite.pixels_rgba.chunks_exact_mut(4) {
        if px[3] < 8 {
            continue;
        }
        let d2 = color_dist_sq(px[0], px[1], px[2], bg);
        if d2 <= threshold {
            px[3] = 0;
        }
    }
}

fn crop_to_alpha(sprite: Sprite) -> Option<Sprite> {
    let w = sprite.width as usize;
    let h = sprite.height as usize;
    let p = &sprite.pixels_rgba;
    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut found = false;

    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            if p[i + 3] > 12 {
                found = true;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if !found {
        return None;
    }

    let new_w = max_x - min_x + 1;
    let new_h = max_y - min_y + 1;
    let mut out = vec![0u8; new_w * new_h * 4];
    for y in 0..new_h {
        for x in 0..new_w {
            let src = ((min_y + y) * w + (min_x + x)) * 4;
            let dst = (y * new_w + x) * 4;
            out[dst..dst + 4].copy_from_slice(&p[src..src + 4]);
        }
    }

    Some(Sprite {
        width: new_w as u32,
        height: new_h as u32,
        pixels_rgba: out,
    })
}

fn resize_nearest(sprite: &Sprite, target_height: u32) -> Sprite {
    let target_height = target_height.max(1);
    let target_width = ((sprite.width as u64 * target_height as u64) / sprite.height as u64)
        .max(1) as u32;
    let mut out = vec![0u8; target_width as usize * target_height as usize * 4];

    for y in 0..target_height as usize {
        let src_y = (y * sprite.height as usize) / target_height as usize;
        for x in 0..target_width as usize {
            let src_x = (x * sprite.width as usize) / target_width as usize;
            let src_i = (src_y * sprite.width as usize + src_x) * 4;
            let dst_i = (y * target_width as usize + x) * 4;
            out[dst_i..dst_i + 4].copy_from_slice(&sprite.pixels_rgba[src_i..src_i + 4]);
        }
    }

    Sprite {
        width: target_width,
        height: target_height,
        pixels_rgba: out,
    }
}

fn load_character_sprite(bytes: &[u8]) -> Option<Sprite> {
    let mut sprite = decode_bmp_to_rgba(bytes)?;
    key_background_transparent(&mut sprite);
    crop_to_alpha(sprite)
}

#[inline]
fn blend_rgba_over(dst: &mut [u8], src: &[u8]) {
    let sa = src[3] as u32;
    if sa == 0 {
        return;
    }
    let inv = 255u32 - sa;
    dst[0] = (((src[0] as u32 * sa) + (dst[0] as u32 * inv)) / 255) as u8;
    dst[1] = (((src[1] as u32 * sa) + (dst[1] as u32 * inv)) / 255) as u8;
    dst[2] = (((src[2] as u32 * sa) + (dst[2] as u32 * inv)) / 255) as u8;
    dst[3] = 255;
}

fn blit_sprite_center_bottom_rgba(
    dest: &mut [u8],
    dest_width: u32,
    dest_height: u32,
    sprite: &Sprite,
    center_x: i32,
    bottom_y: i32,
) {
    let start_x = center_x - (sprite.width as i32 / 2);
    let start_y = bottom_y - sprite.height as i32;

    for sy in 0..(sprite.height as i32) {
        let dy = start_y + sy;
        if !(0..(dest_height as i32)).contains(&dy) {
            continue;
        }
        for sx in 0..(sprite.width as i32) {
            let dx = start_x + sx;
            if !(0..(dest_width as i32)).contains(&dx) {
                continue;
            }
            let src_i = ((sy as usize * sprite.width as usize) + sx as usize) * 4;
            if sprite.pixels_rgba[src_i + 3] == 0 {
                continue;
            }
            let dst_i = (((dy as u32 * dest_width + dx as u32) * 4) as usize) as usize;
            blend_rgba_over(
                &mut dest[dst_i..dst_i + 4],
                &sprite.pixels_rgba[src_i..src_i + 4],
            );
        }
    }
}

fn spawn_anchor_for_dims(width: u32, height: u32, spawn: SelectedSpawn) -> (i32, i32, i32) {
    let sy = height as f64 / REF_LEVEL_HEIGHT;
    let center_x = (width as f64 * 0.5).round() as i32;
    let edge_offset = (BASE_CY_FROM_EDGE_REF * sy).round() as i32;
    let center_y = match spawn {
        SelectedSpawn::Alien => edge_offset,
        SelectedSpawn::Demon => height as i32 - edge_offset,
    };
    let base_radius = (BASE_RADIUS_REF * sy).round().max(10.0) as i32;
    (center_x, center_y, base_radius)
}

fn default_character_position(width: u32, height: u32, spawn: SelectedSpawn) -> (i32, i32) {
    let sy = height as f64 / REF_LEVEL_HEIGHT;
    let (cx, cy, base_radius) = spawn_anchor_for_dims(width, height, spawn);
    let bottom = cy + base_radius - (3.0 * sy).round() as i32;
    (cx, bottom)
}

fn character_position_for_spawn(
    width: u32,
    height: u32,
    spawn: SelectedSpawn,
    stored: Option<(i32, i32)>,
) -> (i32, i32) {
    stored.unwrap_or_else(|| default_character_position(width, height, spawn))
}

fn ensure_character_positions_initialized(state: &mut AppState, width: u32, height: u32) {
    if state.alien_position.is_none() {
        state.alien_position = Some(default_character_position(width, height, SelectedSpawn::Alien));
    }
    if state.demon_position.is_none() {
        state.demon_position = Some(default_character_position(width, height, SelectedSpawn::Demon));
    }
}

fn character_position(state: &AppState, spawn: SelectedSpawn) -> (i32, i32) {
    match spawn {
        SelectedSpawn::Alien => character_position_for_spawn(
            state.content_width.max(1),
            state.content_height.max(1),
            spawn,
            state.alien_position,
        ),
        SelectedSpawn::Demon => character_position_for_spawn(
            state.content_width.max(1),
            state.content_height.max(1),
            spawn,
            state.demon_position,
        ),
    }
}

fn set_character_position(state: &mut AppState, spawn: SelectedSpawn, pos: (i32, i32)) {
    match spawn {
        SelectedSpawn::Alien => state.alien_position = Some(pos),
        SelectedSpawn::Demon => state.demon_position = Some(pos),
    }
}

fn character_sprite_bounds(
    state: &AppState,
    spawn: SelectedSpawn,
    pos: (i32, i32),
    selected: bool,
) -> (f64, f64, f64, f64) {
    let sy = state.content_height as f64 / REF_LEVEL_HEIGHT;
    let scale = if selected {
        SPAWN_SPRITE_SELECT_SCALE
    } else {
        1.0
    };
    let target_h = (SPAWN_SPRITE_H_REF * sy * scale).max(16.0);
    let sprite = match spawn {
        SelectedSpawn::Alien => state.alien_sprite.as_ref(),
        SelectedSpawn::Demon => state.demon_sprite.as_ref(),
    };

    let aspect = sprite
        .map(|s| s.width as f64 / s.height.max(1) as f64)
        .unwrap_or(1.0);
    let target_w = (target_h * aspect).max(12.0);
    let left = pos.0 as f64 - (target_w * 0.5);
    let right = left + target_w;
    let top = pos.1 as f64 - target_h;
    let bottom = pos.1 as f64;
    let pad = (8.0 * sy).max(6.0);
    (left - pad, top - pad, right + pad, bottom + pad)
}

#[inline]
fn point_in_bounds(x: f64, y: f64, bounds: (f64, f64, f64, f64)) -> bool {
    x >= bounds.0 && x <= bounds.2 && y >= bounds.1 && y <= bounds.3
}

fn pointer_to_content_coords(state: &AppState, pointer: &PointerEvent) -> Option<(f64, f64)> {
    let draw = state.draw_info?;
    let rect = state.canvas.get_bounding_client_rect();
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }

    let css_x = pointer.client_x() as f64 - rect.left();
    let css_y = pointer.client_y() as f64 - rect.top();
    if css_x < 0.0 || css_y < 0.0 || css_x > rect.width() || css_y > rect.height() {
        return None;
    }

    let px = css_x * state.canvas.width() as f64 / rect.width();
    let py = css_y * state.canvas.height() as f64 / rect.height();

    if px < draw.x || py < draw.y || px > draw.x + draw.draw_width || py > draw.y + draw.draw_height
    {
        return None;
    }

    let u = (px - draw.x) / draw.draw_width;
    let v = (py - draw.y) / draw.draw_height;
    Some((u * state.content_width as f64, v * state.content_height as f64))
}

fn pick_spawn_at_point(state: &AppState, level_x: f64, level_y: f64) -> Option<SelectedSpawn> {
    if state.content_width == 0 || state.content_height == 0 {
        return None;
    }

    let alien_pos = character_position(state, SelectedSpawn::Alien);
    let demon_pos = character_position(state, SelectedSpawn::Demon);

    // Primary hit target: sprite footprint, so tapping the character itself is reliable.
    let alien_bounds = character_sprite_bounds(
        state,
        SelectedSpawn::Alien,
        alien_pos,
        state.selected_spawn == Some(SelectedSpawn::Alien),
    );
    let demon_bounds = character_sprite_bounds(
        state,
        SelectedSpawn::Demon,
        demon_pos,
        state.selected_spawn == Some(SelectedSpawn::Demon),
    );
    let alien_hit = point_in_bounds(level_x, level_y, alien_bounds);
    let demon_hit = point_in_bounds(level_x, level_y, demon_bounds);

    if alien_hit || demon_hit {
        if alien_hit && !demon_hit {
            return Some(SelectedSpawn::Alien);
        }
        if demon_hit && !alien_hit {
            return Some(SelectedSpawn::Demon);
        }
        // Overlap edge-case: choose nearest base center.
    }

    // Fallback hit target: spawn pad circle.
    let (_, ay, ar) =
        spawn_anchor_for_dims(state.content_width, state.content_height, SelectedSpawn::Alien);
    let (_, dy, dr) =
        spawn_anchor_for_dims(state.content_width, state.content_height, SelectedSpawn::Demon);
    let hit_radius = (ar.max(dr) as f64 * 1.5).max(34.0);
    let hit_r2 = hit_radius * hit_radius;

    let alien_d2 = (level_x - alien_pos.0 as f64).powi(2) + (level_y - ay as f64).powi(2);
    let demon_d2 = (level_x - demon_pos.0 as f64).powi(2) + (level_y - dy as f64).powi(2);

    if alien_d2 <= hit_r2 || demon_d2 <= hit_r2 {
        if alien_d2 <= demon_d2 {
            Some(SelectedSpawn::Alien)
        } else {
            Some(SelectedSpawn::Demon)
        }
    } else {
        None
    }
}

fn lane_target_from_point(state: &AppState, level_x: f64, level_y: f64) -> Option<(usize, i32, i32)> {
    if state.content_width == 0 || state.content_height == 0 {
        return None;
    }

    let sx = state.content_width as f64 / REF_LEVEL_WIDTH;
    let sy = state.content_height as f64 / REF_LEVEL_HEIGHT;
    let lane_top = REF_LANE_TOP * sy;
    let lane_bottom = REF_LANE_BOTTOM * sy;
    if level_y < lane_top || level_y > lane_bottom {
        return None;
    }

    let lane_half = (REF_LANE_WIDTH * sx) * 0.5;
    let lane_hit_pad = (10.0 * sx).max(8.0);
    let mut best_idx: Option<usize> = None;
    let mut best_dist = f64::MAX;

    for (idx, center_ref) in REF_LANE_CENTERS.iter().enumerate() {
        let center = center_ref * sx;
        let dist = (level_x - center).abs();
        if dist <= lane_half + lane_hit_pad && dist < best_dist {
            best_dist = dist;
            best_idx = Some(idx);
        }
    }

    let lane_idx = best_idx?;
    let lane_center_x = (REF_LANE_CENTERS[lane_idx] * sx).round() as i32;
    let y_min = (lane_top + (SPAWN_SPRITE_H_REF * sy * 0.8)).round() as i32;
    let y_max = (lane_bottom - (2.0 * sy)).round() as i32;
    let lane_bottom_y = (level_y.round() as i32).clamp(y_min, y_max.max(y_min));
    Some((lane_idx, lane_center_x, lane_bottom_y))
}

fn overlay_character_sprite(
    pixels_rgba: &mut [u8],
    width: u32,
    height: u32,
    sprite: Option<&Sprite>,
    position: (i32, i32),
    selected: bool,
) {
    let Some(source) = sprite else {
        return;
    };

    let sy = height as f64 / REF_LEVEL_HEIGHT;
    let scale = if selected {
        SPAWN_SPRITE_SELECT_SCALE
    } else {
        1.0
    };
    let target_h = ((SPAWN_SPRITE_H_REF * sy * scale).round() as u32).max(16);
    let scaled = resize_nearest(source, target_h);

    blit_sprite_center_bottom_rgba(
        pixels_rgba,
        width,
        height,
        &scaled,
        position.0,
        position.1,
    );
}

fn upload_level_map_texture_with_characters(
    gl: &Gl,
    texture: &WebGlTexture,
    level: &LevelMap,
    selected: Option<SelectedSpawn>,
    alien_sprite: Option<&Sprite>,
    demon_sprite: Option<&Sprite>,
    alien_position: Option<(i32, i32)>,
    demon_position: Option<(i32, i32)>,
) -> Result<Option<String>, JsValue> {
    let mut composed = level.pixels_rgba.clone();
    let alien_pos =
        character_position_for_spawn(level.width, level.height, SelectedSpawn::Alien, alien_position);
    let demon_pos =
        character_position_for_spawn(level.width, level.height, SelectedSpawn::Demon, demon_position);

    overlay_character_sprite(
        &mut composed,
        level.width,
        level.height,
        alien_sprite,
        alien_pos,
        selected == Some(SelectedSpawn::Alien),
    );
    overlay_character_sprite(
        &mut composed,
        level.width,
        level.height,
        demon_sprite,
        demon_pos,
        selected == Some(SelectedSpawn::Demon),
    );

    let composed_level = LevelMap {
        width: level.width,
        height: level.height,
        pixels_rgba: composed,
    };

    upload_level_map_texture(gl, texture, &composed_level)
}

fn create_webgl_context(canvas: &HtmlCanvasElement) -> Result<Gl, JsValue> {
    // Conservative defaults to reduce GPU work and avoid expensive buffers.
    let options = Object::new();
    Reflect::set(&options, &JsValue::from_str("alpha"), &JsValue::FALSE)?;
    Reflect::set(&options, &JsValue::from_str("antialias"), &JsValue::FALSE)?;
    Reflect::set(&options, &JsValue::from_str("depth"), &JsValue::FALSE)?;
    Reflect::set(&options, &JsValue::from_str("stencil"), &JsValue::FALSE)?;
    Reflect::set(
        &options,
        &JsValue::from_str("premultipliedAlpha"),
        &JsValue::FALSE,
    )?;
    Reflect::set(
        &options,
        &JsValue::from_str("powerPreference"),
        &JsValue::from_str("low-power"),
    )?;

    let options = JsValue::from(options);
    let ctx = canvas
        .get_context_with_context_options("webgl", &options)?
        .or_else(|| canvas.get_context("webgl").ok().flatten())
        .ok_or_else(|| JsValue::from_str("WebGL unavailable"))?;

    ctx.dyn_into::<Gl>()
        .map_err(|_| JsValue::from_str("WebGL context is not a WebGlRenderingContext"))
}

fn get_export_fn(exports: &JsValue, name: &str) -> Result<Function, JsValue> {
    Reflect::get(exports, &JsValue::from_str(name))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str(&format!("Missing export function: {}", name)))
}

fn call_export_u32(func: &Function, name: &str) -> Result<u32, JsValue> {
    let value = func.call0(&JsValue::UNDEFINED)?;
    value
        .as_f64()
        .map(|v| v.max(0.0) as u32)
        .ok_or_else(|| JsValue::from_str(&format!("Export did not return a number: {}", name)))
}

async fn load_level_map_from_wasm(url: &str) -> Result<LevelMap, JsValue> {
    let resp_value = JsFuture::from(window().fetch_with_str(url)).await?;
    let resp: Response = resp_value.dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!(
            "Failed to fetch {} (HTTP {})",
            url,
            resp.status()
        )));
    }

    let buf_value = JsFuture::from(resp.array_buffer()?).await?;
    let buffer: ArrayBuffer = buf_value.dyn_into()?;

    // `js_sys::WebAssembly::instantiate_buffer` takes a Rust byte slice. Copy once from the
    // fetched ArrayBuffer into wasm memory so we can pass `&[u8]`.
    let buf_u8 = Uint8Array::new(&buffer);
    let mut wasm_bytes = vec![0u8; buf_u8.length() as usize];
    buf_u8.copy_to(&mut wasm_bytes);

    let result =
        JsFuture::from(WebAssembly::instantiate_buffer(&wasm_bytes, &Object::new())).await?;
    let instance = Reflect::get(&result, &JsValue::from_str("instance"))?;
    let exports = Reflect::get(&instance, &JsValue::from_str("exports"))?;

    // Initialize/generate pixels inside the level module.
    let init = get_export_fn(&exports, "bvb_level_init")?;
    init.call0(&JsValue::UNDEFINED)?;

    let width_fn = get_export_fn(&exports, "bvb_level_width")?;
    let height_fn = get_export_fn(&exports, "bvb_level_height")?;
    let ptr_fn = get_export_fn(&exports, "bvb_level_pixels_ptr")?;
    let len_fn = get_export_fn(&exports, "bvb_level_pixels_len")?;

    let width = call_export_u32(&width_fn, "bvb_level_width")?;
    let height = call_export_u32(&height_fn, "bvb_level_height")?;
    let ptr = call_export_u32(&ptr_fn, "bvb_level_pixels_ptr")?;
    let len = call_export_u32(&len_fn, "bvb_level_pixels_len")?;

    let expected_len = width.saturating_mul(height).saturating_mul(4).max(1);
    if len != expected_len {
        return Err(JsValue::from_str(&format!(
            "Level pixel buffer has unexpected length: got {}, expected {}",
            len, expected_len
        )));
    }

    let memory = Reflect::get(&exports, &JsValue::from_str("memory"))?;
    let mem_buffer = Reflect::get(&memory, &JsValue::from_str("buffer"))?;
    let mem_u8 = Uint8Array::new(&mem_buffer);

    let start = ptr;
    let end = ptr.saturating_add(len);
    let slice = mem_u8.subarray(start, end);
    let mut pixels = vec![0u8; len as usize];
    slice.copy_to(&mut pixels);

    Ok(LevelMap {
        width,
        height,
        pixels_rgba: pixels,
    })
}

fn upload_level_map_texture(
    gl: &Gl,
    texture: &WebGlTexture,
    level: &LevelMap,
) -> Result<Option<String>, JsValue> {
    gl.bind_texture(Gl::TEXTURE_2D, Some(texture));
    gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1);
    gl.pixel_storei(Gl::UNPACK_FLIP_Y_WEBGL, 0);

    gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        Gl::TEXTURE_2D,
        0,
        Gl::RGBA as i32,
        level.width as i32,
        level.height as i32,
        0,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(&level.pixels_rgba),
    )?;

    Ok(gl_check(gl, "texImage2D(level)"))
}

fn ensure_level_prefetch(state: Rc<RefCell<AppState>>, schedule_redraw: Rc<dyn Fn()>) {
    {
        let mut st = state.borrow_mut();
        match st.level_state {
            LevelLoadState::NotStarted => {
                st.level_state = LevelLoadState::Loading;
                st.last_event = "level_prefetch_start".to_string();
                let _ = update_diagnostics(&st);
            }
            LevelLoadState::Loading | LevelLoadState::Ready(_) => {
                return;
            }
            LevelLoadState::Error(_) => {
                // Allow retry.
                st.level_state = LevelLoadState::Loading;
                st.last_event = "level_prefetch_retry".to_string();
                let _ = update_diagnostics(&st);
            }
        }
    }

    spawn_local(async move {
        let result = load_level_map_from_wasm(LEVEL_WASM_URL).await;

        let should_start_game = {
            let mut st = state.borrow_mut();
            match result {
                Ok(level) => {
                    st.level_state = LevelLoadState::Ready(level);
                    st.last_event = "level_prefetch_ready".to_string();
                }
                Err(err) => {
                    st.level_state = LevelLoadState::Error(js_value_to_string(&err));
                    st.last_event = "level_prefetch_error".to_string();
                }
            }
            let _ = update_diagnostics(&st);
            st.scene == Scene::LoadingLevel
        };

        if should_start_game {
            let mut st = state.borrow_mut();
            let mut maybe_upload: Option<(u32, u32, Result<Option<String>, JsValue>)> = None;
            let mut maybe_error: Option<String> = None;
            match &st.level_state {
                LevelLoadState::Ready(level) => {
                    maybe_upload = Some((
                        level.width.max(1),
                        level.height.max(1),
                        upload_level_map_texture_with_characters(
                            &st.gl,
                            &st.texture,
                            level,
                            st.selected_spawn,
                            st.alien_sprite.as_ref(),
                            st.demon_sprite.as_ref(),
                            st.alien_position,
                            st.demon_position,
                        ),
                    ));
                }
                LevelLoadState::Error(message) => {
                    maybe_error = Some(message.clone());
                }
                _ => {}
            }

            if let Some((width, height, upload)) = maybe_upload {
                match upload {
                    Ok(gl_err) => {
                        st.scene = Scene::Level;
                        st.selected_spawn = None;
                        st.content_width = width;
                        st.content_height = height;
                        ensure_character_positions_initialized(&mut st, width, height);
                        st.last_event = "level_upload".to_string();
                        st.last_gl_error = gl_err;

                        let _ = update_geometry(&mut st);
                        render(&mut st);
                        set_status(&st.document, &st.diagnostics_text, "in_game", "In game");
                        let _ = update_diagnostics(&st);
                    }
                    Err(_) => {
                        set_status(
                            &st.document,
                            &st.diagnostics_text,
                            "error",
                            "Failed to upload level texture",
                        );
                        let _ = update_diagnostics(&st);
                    }
                }
            } else if let Some(message) = maybe_error {
                set_status(&st.document, &st.diagnostics_text, "error", &message);
                let _ = update_diagnostics(&st);
            }
        }

        schedule_redraw();
    });
}

fn set_status(document: &Document, diagnostics_text: &HtmlElement, status: &str, message: &str) {
    if let Some(el) = document.document_element() {
        let _ = el.set_attribute("data-render-status", status);
    }
    diagnostics_text.set_text_content(Some(message));
}

fn set_frame_css_vars(document: &Document, x: i32, y: i32, w: i32, h: i32) {
    let Some(el) = document.document_element() else {
        return;
    };
    let Ok(html_el) = el.dyn_into::<HtmlElement>() else {
        return;
    };

    let style = html_el.style();
    let _ = style.set_property("--frame-x", &format!("{}px", x));
    let _ = style.set_property("--frame-y", &format!("{}px", y));
    let _ = style.set_property("--frame-w", &format!("{}px", w));
    let _ = style.set_property("--frame-h", &format!("{}px", h));
}

fn set_diagnostics_open(document: &Document, state: &mut AppState, open: bool) {
    state.diagnostics_open = open;

    if let Some(el) = document.document_element() {
        let _ = el.set_attribute("data-diag-open", if open { "1" } else { "0" });
    }

    let _ = state
        .tools_button
        .set_attribute("aria-expanded", if open { "true" } else { "false" });
    let _ = state
        .tools_button
        .set_attribute("aria-pressed", if open { "true" } else { "false" });
    let _ = state.tools_button.set_attribute(
        "title",
        if open {
            "Hide diagnostics"
        } else {
            "Diagnostics"
        },
    );
    let _ = state
        .diagnostics
        .set_attribute("aria-hidden", if open { "false" } else { "true" });
}

fn compile_shader(gl: &Gl, shader_type: u32, source: &str) -> Result<WebGlShader, JsValue> {
    let shader = gl
        .create_shader(shader_type)
        .ok_or_else(|| JsValue::from_str("Unable to create shader"))?;
    gl.shader_source(&shader, source);
    gl.compile_shader(&shader);

    if gl
        .get_shader_parameter(&shader, Gl::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(shader)
    } else {
        let info = gl
            .get_shader_info_log(&shader)
            .unwrap_or_else(|| "Unknown shader error".to_string());
        Err(JsValue::from_str(&info))
    }
}

fn create_program(
    gl: &Gl,
    vertex_source: &str,
    fragment_source: &str,
) -> Result<WebGlProgram, JsValue> {
    let vertex_shader = compile_shader(gl, Gl::VERTEX_SHADER, vertex_source)?;
    let fragment_shader = compile_shader(gl, Gl::FRAGMENT_SHADER, fragment_source)?;

    let program = gl
        .create_program()
        .ok_or_else(|| JsValue::from_str("Unable to create program"))?;

    gl.attach_shader(&program, &vertex_shader);
    gl.attach_shader(&program, &fragment_shader);
    gl.link_program(&program);

    if gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .as_bool()
        .unwrap_or(false)
    {
        Ok(program)
    } else {
        let info = gl
            .get_program_info_log(&program)
            .unwrap_or_else(|| "Unknown program error".to_string());
        Err(JsValue::from_str(&info))
    }
}

fn compute_draw_info(
    canvas_width: f64,
    canvas_height: f64,
    content_width: f64,
    content_height: f64,
    scene: Scene,
) -> DrawInfo {
    let (frame_width, frame_height, frame_x, frame_y) = compute_frame(canvas_width, canvas_height);
    let scale = match scene {
        // Title should fill the vertical frame edge-to-edge.
        Scene::Title => (frame_width / content_width).max(frame_height / content_height),
        // Keep level content fully visible.
        Scene::LoadingLevel | Scene::Level => {
            (frame_width / content_width).min(frame_height / content_height)
        }
    };

    let draw_width = content_width * scale;
    let draw_height = content_height * scale;

    let x = frame_x + (frame_width - draw_width) * 0.5;
    let y = frame_y + (frame_height - draw_height) * 0.5;

    DrawInfo {
        frame_width,
        frame_height,
        frame_x,
        frame_y,
        scale,
        draw_width,
        draw_height,
        x,
        y,
    }
}

fn update_geometry(state: &mut AppState) -> Result<(), JsValue> {
    if state.context_lost {
        return Ok(());
    }

    let window = window();
    let dpr = window.device_pixel_ratio().min(2.5);
    let css_width = window.inner_width()?.as_f64().unwrap_or(1.0).max(1.0);
    let css_height = window.inner_height()?.as_f64().unwrap_or(1.0).max(1.0);
    let mut width = (css_width * dpr).max(1.0);
    let mut height = (css_height * dpr).max(1.0);

    // Avoid allocating huge canvases on high-DPI or large displays.
    // This is a title screen; we can cap internal resolution without harming UX.
    let max_rb = state.gl_max_renderbuffer_size.max(1) as f64;
    let max_dim = max_rb.min(4096.0);
    let max_side = width.max(height);
    if max_side > max_dim {
        let scale = max_dim / max_side;
        width *= scale;
        height *= scale;
    }

    let width = width.floor().max(1.0) as u32;
    let height = height.floor().max(1.0) as u32;

    if state.canvas.width() != width {
        state.canvas.set_width(width);
    }
    if state.canvas.height() != height {
        state.canvas.set_height(height);
    }

    state.gl.viewport(0, 0, width as i32, height as i32);

    let scale_x = (width as f64) / css_width;
    let scale_y = (height as f64) / css_height;
    let (frame_w, frame_h, frame_x, frame_y) = compute_frame(width as f64, height as f64);
    let frame_css = (
        (frame_x / scale_x).round() as i32,
        (frame_y / scale_y).round() as i32,
        (frame_w / scale_x).round() as i32,
        (frame_h / scale_y).round() as i32,
    );
    if state.hud_frame_css != Some(frame_css) {
        set_frame_css_vars(
            &state.document,
            frame_css.0,
            frame_css.1,
            frame_css.2,
            frame_css.3,
        );
        state.hud_frame_css = Some(frame_css);
    }

    if state.content_width == 0 || state.content_height == 0 {
        return Ok(());
    }

    let content_width = state.content_width.max(1) as f64;
    let content_height = state.content_height.max(1) as f64;
    let draw_info = compute_draw_info(
        width as f64,
        height as f64,
        content_width,
        content_height,
        state.scene,
    );
    state.draw_info = Some(draw_info);

    let left = (draw_info.x / width as f64) * 2.0 - 1.0;
    let right = ((draw_info.x + draw_info.draw_width) / width as f64) * 2.0 - 1.0;
    let top = 1.0 - (draw_info.y / height as f64) * 2.0;
    let bottom = 1.0 - ((draw_info.y + draw_info.draw_height) / height as f64) * 2.0;

    let positions: [f32; 8] = [
        left as f32,
        top as f32,
        right as f32,
        top as f32,
        left as f32,
        bottom as f32,
        right as f32,
        bottom as f32,
    ];

    let tex_coords: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];

    let positions_array = js_sys::Float32Array::from(positions.as_ref());
    state
        .gl
        .bind_buffer(Gl::ARRAY_BUFFER, Some(&state.position_buffer));
    state.gl.buffer_data_with_array_buffer_view(
        Gl::ARRAY_BUFFER,
        &positions_array,
        Gl::STATIC_DRAW,
    );

    let tex_coords_array = js_sys::Float32Array::from(tex_coords.as_ref());
    state
        .gl
        .bind_buffer(Gl::ARRAY_BUFFER, Some(&state.tex_coord_buffer));
    state.gl.buffer_data_with_array_buffer_view(
        Gl::ARRAY_BUFFER,
        &tex_coords_array,
        Gl::STATIC_DRAW,
    );

    state.last_gl_error = gl_check(&state.gl, "update_geometry");
    Ok(())
}

fn render(state: &mut AppState) {
    if state.context_lost || state.content_width == 0 || state.content_height == 0 {
        return;
    }

    // Be defensive: some implementations reset state on canvas resize.
    state.gl.use_program(Some(&state.program));

    state
        .gl
        .bind_buffer(Gl::ARRAY_BUFFER, Some(&state.position_buffer));
    state.gl.enable_vertex_attrib_array(state.a_position);
    state
        .gl
        .vertex_attrib_pointer_with_i32(state.a_position, 2, Gl::FLOAT, false, 0, 0);

    state
        .gl
        .bind_buffer(Gl::ARRAY_BUFFER, Some(&state.tex_coord_buffer));
    state.gl.enable_vertex_attrib_array(state.a_tex_coord);
    state
        .gl
        .vertex_attrib_pointer_with_i32(state.a_tex_coord, 2, Gl::FLOAT, false, 0, 0);

    state.gl.active_texture(Gl::TEXTURE0);
    state.gl.bind_texture(Gl::TEXTURE_2D, Some(&state.texture));
    state.gl.uniform1i(Some(&state.u_texture), 0);

    state.gl.clear_color(0.03, 0.05, 0.09, 1.0);
    state.gl.clear(Gl::COLOR_BUFFER_BIT);
    state.gl.draw_arrays(Gl::TRIANGLE_STRIP, 0, 4);
    state.last_gl_error = gl_check(&state.gl, "render");
}

fn render_qr_data_url(document: &Document, payload: &str, size_px: u32) -> Result<String, JsValue> {
    let qr = QrCode::encode_text(payload, QrCodeEcc::Medium)
        .map_err(|_| JsValue::from_str("failed to encode LAN QR payload"))?;
    let qr_size = qr.size();
    let border_modules = 2i32;
    let total_modules = qr_size + border_modules * 2;

    let canvas = document
        .create_element("canvas")?
        .dyn_into::<HtmlCanvasElement>()?;
    canvas.set_width(size_px);
    canvas.set_height(size_px);

    let context = canvas
        .get_context("2d")?
        .ok_or_else(|| JsValue::from_str("2D canvas unavailable for LAN QR"))?
        .dyn_into::<CanvasRenderingContext2d>()?;
    context.set_image_smoothing_enabled(false);

    let size = size_px as f64;
    context.set_fill_style_str("#f8fcff");
    context.fill_rect(0.0, 0.0, size, size);

    context.set_fill_style_str("#0a1018");
    let module_px = size / total_modules as f64;

    for y in 0..qr_size {
        for x in 0..qr_size {
            if !qr.get_module(x, y) {
                continue;
            }

            let x0 = ((x + border_modules) as f64 * module_px).floor();
            let y0 = ((y + border_modules) as f64 * module_px).floor();
            let x1 = ((x + border_modules + 1) as f64 * module_px).ceil();
            let y1 = ((y + border_modules + 1) as f64 * module_px).ceil();
            context.fill_rect(x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0));
        }
    }

    canvas.to_data_url_with_type("image/png")
}

fn update_lan_share_qr(state: &AppState) -> Result<(), JsValue> {
    let location = window().location();
    let share_url = location.href()?;
    let host = location.hostname().unwrap_or_default().to_ascii_lowercase();
    let is_loopback = host == "localhost" || host == "127.0.0.1" || host == "::1";

    let mut label = share_url.clone();
    if is_loopback {
        label.push_str(" (loopback host; open via LAN IP to share)");
    }

    if state
        .lan_qr
        .get_attribute("data-qr-payload")
        .as_deref()
        != Some(share_url.as_str())
    {
        match render_qr_data_url(&state.document, &share_url, 176) {
            Ok(qr_data_url) => {
                state.lan_qr.set_src(&qr_data_url);
                let _ = state.lan_qr.set_attribute("data-qr-payload", &share_url);
            }
            Err(err) => {
                label.push_str("\nQR unavailable: ");
                label.push_str(&js_value_to_string(&err));
            }
        }
    }

    state.lan_url.set_text_content(Some(&label));
    let _ = state.lan_share.remove_attribute("hidden");

    Ok(())
}

fn update_diagnostics(state: &AppState) -> Result<(), JsValue> {
    let window = window();
    let dpr = window.device_pixel_ratio().min(2.5);

    let (draw_line, frame_line) = if let Some(draw_info) = state.draw_info {
        (
            format!(
                "draw: {}x{} @ scale {:.3}",
                draw_info.draw_width.round(),
                draw_info.draw_height.round(),
                draw_info.scale
            ),
            format!(
                "frame: {}x{} @ ({}, {})",
                draw_info.frame_width.round(),
                draw_info.frame_height.round(),
                draw_info.frame_x.round(),
                draw_info.frame_y.round()
            ),
        )
    } else {
        (
            "draw: (pending)".to_string(),
            "frame: (pending)".to_string(),
        )
    };

    let status = state
        .document
        .document_element()
        .and_then(|el| el.get_attribute("data-render-status"))
        .unwrap_or_else(|| "unknown".to_string());

    let user_agent = if state.user_agent.is_empty() {
        "(unavailable)".to_string()
    } else {
        let mut ua = state.user_agent.clone();
        if ua.len() > 120 {
            ua.truncate(120);
            ua.push_str("…");
        }
        ua
    };

    let last_gl_error = state.last_gl_error.as_deref().unwrap_or("none").to_string();
    let (sw_supported, sw_controlled, sw_script_url) = sw_control_status();

    let lines = [
        format!("status: {}", status),
        format!("event: {}", state.last_event),
        format!("scene: {}", scene_name(state.scene)),
        format!("selected: {}", selected_spawn_name(state.selected_spawn)),
        format!("alien_pos: {}", position_name(state.alien_position)),
        format!("demon_pos: {}", position_name(state.demon_position)),
        format!("title_loaded: {}", state.title_loaded),
        format!("level: {}", level_state_summary(&state.level_state)),
        format!("content: {}x{}", state.content_width, state.content_height),
        format!("context_lost: {}", state.context_lost),
        format!("diagnostics_open: {}", state.diagnostics_open),
        format!(
            "canvas: {}x{} (dpr {:.2})",
            state.canvas.width(),
            state.canvas.height(),
            dpr
        ),
        format!(
            "viewport: {}x{}",
            window.inner_width()?.as_f64().unwrap_or(0.0).floor(),
            window.inner_height()?.as_f64().unwrap_or(0.0).floor()
        ),
        draw_line,
        frame_line,
        "target aspect: 9:16".to_string(),
        "context opts: low-power, no-AA, no-depth".to_string(),
        format!("sw_supported: {}", sw_supported),
        format!("sw_controlled: {}", sw_controlled),
        format!(
            "sw_script: {}",
            sw_script_url.unwrap_or_else(|| "(none)".to_string())
        ),
        format!(
            "limits: max_tex {} max_rb {}",
            state.gl_max_texture_size, state.gl_max_renderbuffer_size
        ),
        format!("gl: {}", state.gl_version),
        format!("renderer: {}", state.gl_renderer),
        format!("vendor: {}", state.gl_vendor),
        format!("gl_error: {}", last_gl_error),
        format!("ua: {}", user_agent),
    ];

    state.diagnostics_text.set_text_content(Some(&lines.join("\n")));
    let _ = update_lan_share_qr(state);

    Ok(())
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();

    if let Err(err) = start_impl() {
        let message = format!("fatal: {}", js_value_to_string(&err));

        if let Some(win) = web_sys::window() {
            if let Some(doc) = win.document() {
                if let Some(el) = doc.document_element() {
                    let _ = el.set_attribute("data-render-status", "error");
                }
                if let Some(diag) = doc.get_element_by_id("diagnostics-text") {
                    diag.set_text_content(Some(&message));
                } else if let Some(diag) = doc.get_element_by_id("diagnostics") {
                    diag.set_text_content(Some(&message));
                }
                if let Some(fallback) = doc.get_element_by_id("fallback") {
                    let _ = fallback.remove_attribute("hidden");
                }
            }
        }

        web_sys::console::error_1(&err);
    }
}

fn start_impl() -> Result<(), JsValue> {
    let win = window();
    let document = win.document().expect("missing document");

    let canvas = document
        .get_element_by_id("gl-canvas")
        .ok_or_else(|| JsValue::from_str("Missing canvas"))?
        .dyn_into::<HtmlCanvasElement>()?;

    let tools_button = document
        .get_element_by_id("tools-button")
        .ok_or_else(|| JsValue::from_str("Missing tools button"))?
        .dyn_into::<HtmlButtonElement>()?;

    let diagnostics = document
        .get_element_by_id("diagnostics")
        .ok_or_else(|| JsValue::from_str("Missing diagnostics"))?
        .dyn_into::<HtmlDivElement>()?;
    let diagnostics_text = document
        .get_element_by_id("diagnostics-text")
        .ok_or_else(|| JsValue::from_str("Missing diagnostics text"))?
        .dyn_into::<HtmlElement>()?;
    let lan_share = document
        .get_element_by_id("lan-share")
        .ok_or_else(|| JsValue::from_str("Missing LAN share panel"))?
        .dyn_into::<HtmlDivElement>()?;
    let lan_qr = document
        .get_element_by_id("lan-qr")
        .ok_or_else(|| JsValue::from_str("Missing LAN QR image"))?
        .dyn_into::<HtmlImageElement>()?;
    let lan_url = document
        .get_element_by_id("lan-url")
        .ok_or_else(|| JsValue::from_str("Missing LAN URL label"))?
        .dyn_into::<HtmlElement>()?;

    let fallback = document
        .get_element_by_id("fallback")
        .and_then(|el| el.dyn_into::<HtmlDivElement>().ok());

    let gl = create_webgl_context(&canvas)?;

    let gl_version = gl
        .get_parameter(Gl::VERSION)?
        .as_string()
        .unwrap_or_else(|| "(unknown)".to_string());
    let gl_renderer = gl
        .get_parameter(Gl::RENDERER)?
        .as_string()
        .unwrap_or_else(|| "(unknown)".to_string());
    let gl_vendor = gl
        .get_parameter(Gl::VENDOR)?
        .as_string()
        .unwrap_or_else(|| "(unknown)".to_string());
    let gl_max_texture_size = gl
        .get_parameter(Gl::MAX_TEXTURE_SIZE)?
        .as_f64()
        .unwrap_or(0.0) as i32;
    let gl_max_renderbuffer_size = gl
        .get_parameter(Gl::MAX_RENDERBUFFER_SIZE)?
        .as_f64()
        .unwrap_or(0.0) as i32;

    let user_agent = win.navigator().user_agent().unwrap_or_default();
    let is_headless = user_agent.to_ascii_lowercase().contains("headless");

    let program = create_program(&gl, VERTEX_SHADER_SOURCE, FRAGMENT_SHADER_SOURCE)?;
    let position_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("Unable to create position buffer"))?;
    let tex_coord_buffer = gl
        .create_buffer()
        .ok_or_else(|| JsValue::from_str("Unable to create tex coord buffer"))?;

    gl.use_program(Some(&program));

    let a_position = gl.get_attrib_location(&program, "a_position");
    if a_position < 0 {
        return Err(JsValue::from_str("Missing a_position attribute"));
    }
    let a_position = a_position as u32;

    let a_tex_coord = gl.get_attrib_location(&program, "a_texCoord");
    if a_tex_coord < 0 {
        return Err(JsValue::from_str("Missing a_texCoord attribute"));
    }
    let a_tex_coord = a_tex_coord as u32;

    let u_texture = gl
        .get_uniform_location(&program, "u_texture")
        .ok_or_else(|| JsValue::from_str("Missing u_texture uniform"))?;
    gl.uniform1i(Some(&u_texture), 0);

    let texture = gl
        .create_texture()
        .ok_or_else(|| JsValue::from_str("Unable to create texture"))?;
    gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::NEAREST as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::NEAREST as i32);

    let image = HtmlImageElement::new()?;
    let alien_sprite = load_character_sprite(ALIEN_SPRITE_BMP);
    let demon_sprite = load_character_sprite(DEMON_SPRITE_BMP);

    let state = Rc::new(RefCell::new(AppState {
        gl,
        program,
        position_buffer,
        tex_coord_buffer,
        texture,
        a_position,
        a_tex_coord,
        u_texture,
        canvas,
        diagnostics,
        diagnostics_text,
        lan_share,
        lan_qr,
        lan_url,
        tools_button,
        diagnostics_open: false,
        fallback,
        image: image.clone(),
        title_loaded: false,
        scene: Scene::Title,
        content_width: 0,
        content_height: 0,
        level_state: LevelLoadState::NotStarted,
        selected_spawn: None,
        alien_position: None,
        demon_position: None,
        alien_sprite,
        demon_sprite,
        context_lost: false,
        draw_info: None,
        hud_frame_css: None,
        document: document.clone(),
        user_agent,
        gl_version,
        gl_renderer,
        gl_vendor,
        gl_max_texture_size,
        gl_max_renderbuffer_size,
        last_gl_error: None,
        last_event: "init".to_string(),
    }));

    {
        let mut state = state.borrow_mut();
        set_diagnostics_open(&document, &mut state, is_headless);
        state.last_event = "sw_register_pending".to_string();
        let _ = update_geometry(&mut state);
    }

    set_status(
        &document,
        &state.borrow().diagnostics_text,
        "loading",
        "Loading title screen…",
    );
    let _ = update_diagnostics(&state.borrow());

    let state_toggle = Rc::clone(&state);
    let toggle = Closure::wrap(Box::new(move |_event: Event| {
        let document = window().document().expect("missing document");
        let mut state = state_toggle.borrow_mut();
        state.last_event = "toggle_diagnostics".to_string();
        let open = !state.diagnostics_open;
        set_diagnostics_open(&document, &mut state, open);
        let _ = update_diagnostics(&state);
    }) as Box<dyn FnMut(_)>);

    state
        .borrow()
        .tools_button
        .add_event_listener_with_callback("click", toggle.as_ref().unchecked_ref())?;
    toggle.forget();

    let raf_holder: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let schedule_redraw: Rc<dyn Fn()> = {
        let state = Rc::clone(&state);
        let raf_holder = Rc::clone(&raf_holder);
        Rc::new(move || {
            if raf_holder.borrow().is_some() {
                return;
            }

            let state_cb = Rc::clone(&state);
            let raf_holder_cb = Rc::clone(&raf_holder);
            let cb = Closure::wrap(Box::new(move |_ts: f64| {
                raf_holder_cb.borrow_mut().take();

                let mut state = state_cb.borrow_mut();
                if state.context_lost {
                    return;
                }

                if update_geometry(&mut state).is_ok() {
                    render(&mut state);
                    let _ = update_diagnostics(&state);
                }
            }) as Box<dyn FnMut(f64)>);

            if window()
                .request_animation_frame(cb.as_ref().unchecked_ref())
                .is_ok()
            {
                *raf_holder.borrow_mut() = Some(cb);
            }
        })
    };

    {
        let state_sw = Rc::clone(&state);
        let schedule_redraw_sw = Rc::clone(&schedule_redraw);
        spawn_local(async move {
            let event = match register_service_worker_async().await {
                Ok(details) => details,
                Err(reason) => format!("sw_register_skip ({})", reason),
            };

            let mut state = state_sw.borrow_mut();
            state.last_event = event;
            let _ = update_diagnostics(&state);
            drop(state);
            schedule_redraw_sw();
        });
    }

    let state_pointer = Rc::clone(&state);
    let schedule_redraw_pointer = Rc::clone(&schedule_redraw);
    let on_pointerdown = Closure::wrap(Box::new(move |event: Event| {
        event.prevent_default();

        // Canvas input: tap/click anywhere (outside HUD controls) starts the game.
        // HUD elements are siblings of the canvas so they won't target this handler.
        let mut st = state_pointer.borrow_mut();
        if st.context_lost {
            return;
        }

        st.last_event = "pointerdown".to_string();

        if st.scene == Scene::Level {
            let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
                let _ = update_diagnostics(&st);
                return;
            };

            let coords = pointer_to_content_coords(&st, pointer);
            if let Some((level_x, level_y)) = coords {
                if let Some(picked) = pick_spawn_at_point(&st, level_x, level_y) {
                    let previous = st.selected_spawn;
                    st.selected_spawn = Some(picked);
                    st.last_event = match (previous, Some(picked)) {
                        (Some(SelectedSpawn::Alien), Some(SelectedSpawn::Demon)) => {
                            "select_swap_alien_to_demon".to_string()
                        }
                        (Some(SelectedSpawn::Demon), Some(SelectedSpawn::Alien)) => {
                            "select_swap_demon_to_alien".to_string()
                        }
                        (_, Some(SelectedSpawn::Alien)) => "select_alien".to_string(),
                        (_, Some(SelectedSpawn::Demon)) => "select_demon".to_string(),
                        _ => "select_none".to_string(),
                    };
                } else if let Some(selected) = st.selected_spawn {
                    if let Some((lane_idx, lane_x, lane_bottom_y)) =
                        lane_target_from_point(&st, level_x, level_y)
                    {
                        set_character_position(&mut st, selected, (lane_x, lane_bottom_y));
                        st.last_event = format!(
                            "move_{}_lane{}",
                            selected_spawn_name(Some(selected)),
                            lane_idx + 1
                        );
                    } else {
                        st.selected_spawn = None;
                        st.last_event = "select_none".to_string();
                    }
                } else {
                    st.selected_spawn = None;
                    st.last_event = "select_none".to_string();
                }
            } else {
                st.selected_spawn = None;
                st.last_event = "select_none".to_string();
            }

            if let LevelLoadState::Ready(level) = &st.level_state {
                let upload = upload_level_map_texture_with_characters(
                    &st.gl,
                    &st.texture,
                    level,
                    st.selected_spawn,
                    st.alien_sprite.as_ref(),
                    st.demon_sprite.as_ref(),
                    st.alien_position,
                    st.demon_position,
                );
                match upload {
                    Ok(gl_err) => {
                        st.last_gl_error = gl_err;
                        render(&mut st);
                        let _ = update_diagnostics(&st);
                        drop(st);
                        schedule_redraw_pointer();
                    }
                    Err(_) => {
                        set_status(
                            &st.document,
                            &st.diagnostics_text,
                            "error",
                            "Failed to update selected character",
                        );
                        let _ = update_diagnostics(&st);
                    }
                }
            } else {
                let _ = update_diagnostics(&st);
            }
            return;
        }

        if !st.title_loaded || st.scene != Scene::Title {
            let _ = update_diagnostics(&st);
            return;
        }

        let maybe_upload = match &st.level_state {
            LevelLoadState::Ready(level) => Some((
                level.width.max(1),
                level.height.max(1),
                upload_level_map_texture_with_characters(
                    &st.gl,
                    &st.texture,
                    level,
                    st.selected_spawn,
                    st.alien_sprite.as_ref(),
                    st.demon_sprite.as_ref(),
                    st.alien_position,
                    st.demon_position,
                ),
            )),
            LevelLoadState::Loading => {
                st.scene = Scene::LoadingLevel;
                st.last_event = "start_loading_level".to_string();
                set_status(
                    &st.document,
                    &st.diagnostics_text,
                    "loading_level",
                    "Loading level…",
                );
                let _ = update_diagnostics(&st);
                drop(st);
                schedule_redraw_pointer();
                return;
            }
            LevelLoadState::NotStarted | LevelLoadState::Error(_) => {
                st.scene = Scene::LoadingLevel;
                st.last_event = "start_prefetch_level".to_string();
                set_status(
                    &st.document,
                    &st.diagnostics_text,
                    "loading_level",
                    "Loading level…",
                );
                let _ = update_diagnostics(&st);
                drop(st);
                ensure_level_prefetch(
                    Rc::clone(&state_pointer),
                    Rc::clone(&schedule_redraw_pointer),
                );
                return;
            }
        };

        if let Some((width, height, upload)) = maybe_upload {
            match upload {
                Ok(gl_err) => {
                    st.scene = Scene::Level;
                    st.selected_spawn = None;
                    st.content_width = width;
                    st.content_height = height;
                    ensure_character_positions_initialized(&mut st, width, height);
                    st.last_event = "level_upload".to_string();
                    st.last_gl_error = gl_err;

                    let _ = update_geometry(&mut st);
                    render(&mut st);
                    set_status(&st.document, &st.diagnostics_text, "in_game", "In game");
                    let _ = update_diagnostics(&st);
                    drop(st);
                    schedule_redraw_pointer();
                }
                Err(_) => {
                    set_status(
                        &st.document,
                        &st.diagnostics_text,
                        "error",
                        "Failed to upload level texture",
                    );
                    let _ = update_diagnostics(&st);
                }
            }
        }
    }) as Box<dyn FnMut(_)>);

    state
        .borrow()
        .canvas
        .add_event_listener_with_callback("pointerdown", on_pointerdown.as_ref().unchecked_ref())?;
    on_pointerdown.forget();

    let state_ctxlost = Rc::clone(&state);
    let on_ctxlost = Closure::wrap(Box::new(move |event: Event| {
        event.prevent_default();

        let mut state = state_ctxlost.borrow_mut();
        state.context_lost = true;
        state.last_event = "webglcontextlost".to_string();

        if let Some(fallback) = &state.fallback {
            let _ = fallback.remove_attribute("hidden");
        }

        set_status(
            &state.document,
            &state.diagnostics_text,
            "context_lost",
            "WebGL context lost (try reloading)",
        );
        let _ = update_diagnostics(&state);
    }) as Box<dyn FnMut(_)>);

    state.borrow().canvas.add_event_listener_with_callback(
        "webglcontextlost",
        on_ctxlost.as_ref().unchecked_ref(),
    )?;
    on_ctxlost.forget();

    let schedule_redraw_onload = Rc::clone(&schedule_redraw);
    let state_onload = Rc::clone(&state);
    let onload = Closure::wrap(Box::new(move || {
        let mut state = state_onload.borrow_mut();
        state.title_loaded = true;
        state.last_event = "image_onload".to_string();
        state.content_width = state.image.natural_width().max(1);
        state.content_height = state.image.natural_height().max(1);

        state.gl.bind_texture(Gl::TEXTURE_2D, Some(&state.texture));
        state.gl.pixel_storei(Gl::UNPACK_ALIGNMENT, 1);
        state.gl.pixel_storei(Gl::UNPACK_FLIP_Y_WEBGL, 0);

        let upload = state.gl.tex_image_2d_with_u32_and_u32_and_image(
            Gl::TEXTURE_2D,
            0,
            Gl::RGBA as i32,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            &state.image,
        );

        match upload {
            Ok(()) => {
                state.last_gl_error = gl_check(&state.gl, "texImage2D");
                let _ = update_geometry(&mut state);
                render(&mut state);
                set_status(&state.document, &state.diagnostics_text, "ready", "Tap to start");
                let _ = update_diagnostics(&state);
                drop(state);
                ensure_level_prefetch(Rc::clone(&state_onload), Rc::clone(&schedule_redraw_onload));
                schedule_redraw_onload();
            }
            Err(err) => {
                state.last_gl_error = Some(format!("texImage2D: {}", js_value_to_string(&err)));
                if let Some(fallback) = &state.fallback {
                    let _ = fallback.remove_attribute("hidden");
                }
                set_status(
                    &state.document,
                    &state.diagnostics_text,
                    "error",
                    "Failed to upload texture",
                );
                let _ = update_diagnostics(&state);
            }
        }
    }) as Box<dyn FnMut()>);

    image.set_onload(Some(onload.as_ref().unchecked_ref()));
    onload.forget();

    let state_onerror = Rc::clone(&state);
    let onerror = Closure::wrap(Box::new(move || {
        let mut state = state_onerror.borrow_mut();
        state.last_event = "image_onerror".to_string();
        if let Some(fallback) = &state.fallback {
            let _ = fallback.remove_attribute("hidden");
        }
        set_status(
            &state.document,
            &state.diagnostics_text,
            "error",
            "Failed to load title_screen.png",
        );
        let _ = update_diagnostics(&state);
    }) as Box<dyn FnMut()>);

    image.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    onerror.forget();

    image.set_src("assets/title_screen/title_screen.png");

    let state_resize = Rc::clone(&state);
    let schedule_redraw_resize = Rc::clone(&schedule_redraw);
    let resize_timer_handle: Rc<RefCell<Option<i32>>> = Rc::new(RefCell::new(None));

    let state_resize_settle = Rc::clone(&state_resize);
    let schedule_redraw_settle = Rc::clone(&schedule_redraw_resize);
    let resize_settle_cb: Rc<Closure<dyn FnMut()>> = Rc::new(Closure::wrap(Box::new(move || {
        let mut state = state_resize_settle.borrow_mut();
        state.last_event = "resize_settled".to_string();
        drop(state);
        schedule_redraw_settle();
    })
        as Box<dyn FnMut()>));

    let resize_timer_handle_ev = Rc::clone(&resize_timer_handle);
    let resize_settle_cb_ev = Rc::clone(&resize_settle_cb);
    let resize = Closure::wrap(Box::new(move |_event: Event| {
        let mut state = state_resize.borrow_mut();
        state.last_event = "resize_event".to_string();
        drop(state);

        if let Some(id) = resize_timer_handle_ev.borrow_mut().take() {
            window().clear_timeout_with_handle(id);
        }

        match window().set_timeout_with_callback_and_timeout_and_arguments_0(
            resize_settle_cb_ev.as_ref().as_ref().unchecked_ref(),
            140,
        ) {
            Ok(id) => {
                *resize_timer_handle_ev.borrow_mut() = Some(id);
            }
            Err(_) => {
                schedule_redraw_resize();
            }
        }
    }) as Box<dyn FnMut(_)>);

    win.add_event_listener_with_callback("resize", resize.as_ref().unchecked_ref())?;
    resize.forget();

    Ok(())
}
