use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlCanvasElement, WebGlRenderingContext as Gl};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn webgl_context_available() {
    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");
    let canvas = document
        .create_element("canvas")
        .expect("create canvas")
        .dyn_into::<HtmlCanvasElement>()
        .expect("canvas element");

    let gl = canvas
        .get_context("webgl")
        .expect("get context")
        .expect("webgl context")
        .dyn_into::<Gl>()
        .expect("cast webgl");

    assert!(gl.get_error() == Gl::NO_ERROR);
}

#[wasm_bindgen_test]
fn webgl_can_upload_texture() {
    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");
    let canvas = document
        .create_element("canvas")
        .expect("create canvas")
        .dyn_into::<HtmlCanvasElement>()
        .expect("canvas element");

    let gl = canvas
        .get_context("webgl")
        .expect("get context")
        .expect("webgl context")
        .dyn_into::<Gl>()
        .expect("cast webgl");

    let texture = gl.create_texture().expect("create texture");
    gl.bind_texture(Gl::TEXTURE_2D, Some(&texture));
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_S, Gl::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_WRAP_T, Gl::CLAMP_TO_EDGE as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MIN_FILTER, Gl::NEAREST as i32);
    gl.tex_parameteri(Gl::TEXTURE_2D, Gl::TEXTURE_MAG_FILTER, Gl::NEAREST as i32);

    let pixel = [255u8, 0, 0, 255];
    let result = gl.tex_image_2d_with_i32_and_i32_and_i32_and_format_and_type_and_opt_u8_array(
        Gl::TEXTURE_2D,
        0,
        Gl::RGBA as i32,
        1,
        1,
        0,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(&pixel),
    );

    assert!(result.is_ok());
    assert!(gl.get_error() == Gl::NO_ERROR);
}
