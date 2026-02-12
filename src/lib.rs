use js_sys::{Array, ArrayBuffer, Function, Object, Promise, Reflect, Uint8Array, WebAssembly};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    Document, Event, HtmlButtonElement, HtmlCanvasElement, HtmlDivElement, HtmlElement,
    HtmlImageElement, Response, WebGlBuffer, WebGlProgram, WebGlRenderingContext as Gl,
    WebGlShader, WebGlTexture, WebGlUniformLocation, Window,
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

const LEVEL_WASM_URL: &str = "assets/levels/mall_parking_lot.wasm";
const SW_BOOTSTRAP_URL: &str = "/sw_bootstrap_sync.js?v=sync-init-5";

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
    tools_button: HtmlButtonElement,
    diagnostics_open: bool,
    fallback: Option<HtmlDivElement>,
    image: HtmlImageElement,
    title_loaded: bool,
    scene: Scene,
    content_width: u32,
    content_height: u32,
    level_state: LevelLoadState,
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
                        upload_level_map_texture(&st.gl, &st.texture, level),
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
                        st.content_width = width;
                        st.content_height = height;
                        st.last_event = "level_upload".to_string();
                        st.last_gl_error = gl_err;

                        let _ = update_geometry(&mut st);
                        render(&mut st);
                        set_status(&st.document, &st.diagnostics, "in_game", "In game");
                        let _ = update_diagnostics(&st);
                    }
                    Err(_) => {
                        set_status(
                            &st.document,
                            &st.diagnostics,
                            "error",
                            "Failed to upload level texture",
                        );
                        let _ = update_diagnostics(&st);
                    }
                }
            } else if let Some(message) = maybe_error {
                set_status(&st.document, &st.diagnostics, "error", &message);
                let _ = update_diagnostics(&st);
            }
        }

        schedule_redraw();
    });
}

fn set_status(document: &Document, diagnostics: &HtmlDivElement, status: &str, message: &str) {
    if let Some(el) = document.document_element() {
        let _ = el.set_attribute("data-render-status", status);
    }
    diagnostics.set_text_content(Some(message));
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

    state.diagnostics.set_text_content(Some(&lines.join("\n")));

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
                if let Some(diag) = doc.get_element_by_id("diagnostics") {
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
        tools_button,
        diagnostics_open: false,
        fallback,
        image: image.clone(),
        title_loaded: false,
        scene: Scene::Title,
        content_width: 0,
        content_height: 0,
        level_state: LevelLoadState::NotStarted,
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
        &state.borrow().diagnostics,
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

        if !st.title_loaded || st.scene != Scene::Title {
            let _ = update_diagnostics(&st);
            return;
        }

        let maybe_upload = match &st.level_state {
            LevelLoadState::Ready(level) => Some((
                level.width.max(1),
                level.height.max(1),
                upload_level_map_texture(&st.gl, &st.texture, level),
            )),
            LevelLoadState::Loading => {
                st.scene = Scene::LoadingLevel;
                st.last_event = "start_loading_level".to_string();
                set_status(
                    &st.document,
                    &st.diagnostics,
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
                    &st.diagnostics,
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
                    st.content_width = width;
                    st.content_height = height;
                    st.last_event = "level_upload".to_string();
                    st.last_gl_error = gl_err;

                    let _ = update_geometry(&mut st);
                    render(&mut st);
                    set_status(&st.document, &st.diagnostics, "in_game", "In game");
                    let _ = update_diagnostics(&st);
                    drop(st);
                    schedule_redraw_pointer();
                }
                Err(_) => {
                    set_status(
                        &st.document,
                        &st.diagnostics,
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
            &state.diagnostics,
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
                set_status(&state.document, &state.diagnostics, "ready", "Tap to start");
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
                    &state.diagnostics,
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
            &state.diagnostics,
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
