use js_sys::{Object, Reflect};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::{
    Document, Event, HtmlButtonElement, HtmlCanvasElement, HtmlDivElement, HtmlImageElement,
    WebGlBuffer, WebGlProgram, WebGlRenderingContext as Gl, WebGlShader, WebGlTexture,
    WebGlUniformLocation, Window,
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
    image_loaded: bool,
    context_lost: bool,
    draw_info: Option<DrawInfo>,
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

fn set_status(document: &Document, diagnostics: &HtmlDivElement, status: &str, message: &str) {
    if let Some(el) = document.document_element() {
        let _ = el.set_attribute("data-render-status", status);
    }
    diagnostics.set_text_content(Some(message));
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
    image_width: f64,
    image_height: f64,
) -> DrawInfo {
    let target_aspect = 9.0 / 16.0;
    let canvas_aspect = canvas_width / canvas_height;

    let (frame_width, frame_height) = if canvas_aspect > target_aspect {
        (canvas_height * target_aspect, canvas_height)
    } else {
        (canvas_width, canvas_width / target_aspect)
    };

    let frame_x = (canvas_width - frame_width) * 0.5;
    let frame_y = (canvas_height - frame_height) * 0.5;

    let max_width = frame_width * 0.92;
    let max_height = frame_height * 0.86;
    let scale = (max_width / image_width).min(max_height / image_height);

    let draw_width = image_width * scale;
    let draw_height = image_height * scale;

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
    if state.context_lost || !state.image_loaded {
        return Ok(());
    }

    let window = window();
    let dpr = window.device_pixel_ratio().min(2.5);
    let width = window.inner_width()?.as_f64().unwrap_or(1.0).max(1.0) * dpr;
    let height = window.inner_height()?.as_f64().unwrap_or(1.0).max(1.0) * dpr;

    // Avoid allocating huge canvases on high-DPI or large displays.
    // This is a title screen; we can cap internal resolution without harming UX.
    let max_rb = state.gl_max_renderbuffer_size.max(1) as u32;
    let max_dim = max_rb.min(4096);
    let width = (width.floor() as u32).clamp(1, max_dim);
    let height = (height.floor() as u32).clamp(1, max_dim);

    if state.canvas.width() != width {
        state.canvas.set_width(width);
    }
    if state.canvas.height() != height {
        state.canvas.set_height(height);
    }

    state.gl.viewport(0, 0, width as i32, height as i32);

    let image_width = state.image.natural_width().max(1) as f64;
    let image_height = state.image.natural_height().max(1) as f64;

    let draw_info = compute_draw_info(width as f64, height as f64, image_width, image_height);
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
    if state.context_lost || !state.image_loaded {
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
    let image_width = state.image.natural_width();
    let image_height = state.image.natural_height();

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

    let lines = [
        format!("status: {}", status),
        format!("event: {}", state.last_event),
        format!("image_loaded: {}", state.image_loaded),
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
        format!("image: {}x{}", image_width, image_height),
        draw_line,
        frame_line,
        "target aspect: 9:16".to_string(),
        "context opts: low-power, no-AA, no-depth".to_string(),
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
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::LINEAR as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::LINEAR as i32);

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
        image_loaded: false,
        context_lost: false,
        draw_info: None,
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
                if state.context_lost || !state.image_loaded {
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
        state.image_loaded = true;
        state.last_event = "image_onload".to_string();

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
                set_status(&state.document, &state.diagnostics, "ready", "Ready");
                let _ = update_diagnostics(&state);
                drop(state);
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
    let resize = Closure::wrap(Box::new(move |_event: Event| {
        let mut state = state_resize.borrow_mut();
        state.last_event = "resize".to_string();
        drop(state);
        schedule_redraw_resize();
    }) as Box<dyn FnMut(_)>);

    win.add_event_listener_with_callback("resize", resize.as_ref().unchecked_ref())?;
    resize.forget();

    Ok(())
}
