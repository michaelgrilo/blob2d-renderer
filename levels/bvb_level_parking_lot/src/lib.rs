// This crate compiles to a standalone `.wasm` module that exports a pixel-art level map.
//
// It intentionally uses a simple C-style ABI so the main app can instantiate it with
// `WebAssembly.instantiate` and read pixels from this module's linear memory.

const WIDTH: u32 = 288; // 18 tiles * 16 px (9:16)
const HEIGHT: u32 = 512; // 32 tiles * 16 px
const CHANNELS: u32 = 4;

static mut PIXELS_PTR: *mut u8 = core::ptr::null_mut();
static mut PIXELS_LEN: usize = 0;

#[inline]
fn rgba(r: u8, g: u8, b: u8) -> [u8; 4] {
    [r, g, b, 255]
}

#[inline]
fn put(pixels: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    if x < 0 || y < 0 {
        return;
    }
    let x = x as u32;
    let y = y as u32;
    if x >= WIDTH || y >= HEIGHT {
        return;
    }
    let idx = ((y * WIDTH + x) * CHANNELS) as usize;
    pixels[idx..idx + 4].copy_from_slice(&c);
}

fn fill_rect(pixels: &mut [u8], x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
    for yy in y..(y + h) {
        for xx in x..(x + w) {
            put(pixels, xx, yy, c);
        }
    }
}

fn rect_outline(pixels: &mut [u8], x: i32, y: i32, w: i32, h: i32, c: [u8; 4]) {
    for xx in x..(x + w) {
        put(pixels, xx, y, c);
        put(pixels, xx, y + h - 1, c);
    }
    for yy in y..(y + h) {
        put(pixels, x, yy, c);
        put(pixels, x + w - 1, yy, c);
    }
}

fn draw_car(pixels: &mut [u8], x: i32, y: i32, body: [u8; 4]) {
    // Simple 14x22 top-down car sprite.
    let w = 14;
    let h = 22;
    let outline = rgba(10, 14, 20);
    let glass = rgba(44, 92, 128);
    let highlight = rgba(
        body[0].saturating_add(22),
        body[1].saturating_add(22),
        body[2].saturating_add(22),
    );

    fill_rect(pixels, x, y, w, h, body);
    rect_outline(pixels, x, y, w, h, outline);

    // Hood/roof highlight.
    fill_rect(pixels, x + 2, y + 2, w - 4, 3, highlight);

    // Windshields.
    fill_rect(pixels, x + 2, y + 6, w - 4, 4, glass);
    fill_rect(pixels, x + 2, y + h - 10, w - 4, 4, glass);

    // Tail lights.
    put(pixels, x + 2, y + h - 2, rgba(210, 52, 44));
    put(pixels, x + w - 3, y + h - 2, rgba(210, 52, 44));
}

fn generate() -> Vec<u8> {
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * CHANNELS) as usize];

    // Palette-ish colors (limited range, SNES-y).
    let asphalt_a = rgba(18, 26, 36);
    let asphalt_b = rgba(22, 32, 44);
    let asphalt_c = rgba(28, 40, 54);
    let line_white = rgba(214, 224, 232);
    let line_yellow = rgba(214, 186, 62);
    let curb = rgba(96, 106, 116);
    let grass_a = rgba(40, 112, 62);
    let grass_b = rgba(32, 98, 54);
    let building = rgba(174, 152, 124);
    let building_shadow = rgba(138, 118, 98);
    let window = rgba(32, 58, 84);
    let window_glint = rgba(74, 132, 176);

    // Base asphalt with gentle dithering.
    for y in 0..HEIGHT as i32 {
        for x in 0..WIDTH as i32 {
            let pick = ((x ^ y) & 3) as i32;
            let c = match pick {
                0 => asphalt_a,
                1 => asphalt_b,
                _ => asphalt_c,
            };
            put(&mut pixels, x, y, c);
        }
    }

    // Grass borders (thin).
    fill_rect(&mut pixels, 0, (HEIGHT as i32) - 20, WIDTH as i32, 20, grass_a);
    for x in 0..WIDTH as i32 {
        let alt = if (x & 1) == 0 { grass_b } else { grass_a };
        put(&mut pixels, x, (HEIGHT as i32) - 20, alt);
        put(&mut pixels, x, (HEIGHT as i32) - 1, alt);
    }

    // Mall facade at the top.
    let facade_h = 88;
    fill_rect(&mut pixels, 0, 0, WIDTH as i32, facade_h, building);
    fill_rect(&mut pixels, 0, facade_h - 10, WIDTH as i32, 10, building_shadow);
    rect_outline(&mut pixels, 0, 0, WIDTH as i32, facade_h, rgba(62, 52, 44));

    // Windows (simple pattern).
    let mut wx = 16;
    while wx < (WIDTH as i32) - 24 {
        fill_rect(&mut pixels, wx, 18, 20, 14, window);
        rect_outline(&mut pixels, wx, 18, 20, 14, rgba(16, 24, 34));
        fill_rect(&mut pixels, wx + 2, 20, 6, 2, window_glint);
        wx += 28;
    }

    // Entrance.
    let entrance_w = 68;
    let entrance_x = (WIDTH as i32 - entrance_w) / 2;
    fill_rect(&mut pixels, entrance_x, 46, entrance_w, 34, rgba(120, 110, 98));
    rect_outline(&mut pixels, entrance_x, 46, entrance_w, 34, rgba(44, 38, 32));
    fill_rect(&mut pixels, entrance_x + 6, 52, entrance_w - 12, 26, rgba(20, 28, 40));

    // Sidewalk / curb line under facade.
    let curb_y = facade_h + 6;
    fill_rect(&mut pixels, 0, facade_h, WIDTH as i32, 16, curb);
    fill_rect(&mut pixels, 0, curb_y, WIDTH as i32, 2, line_yellow);

    // Parking rows.
    let lot_top = facade_h + 18;
    let lot_bottom = (HEIGHT as i32) - 26;
    let row_height = 52;
    let spot_w = 18;
    let row_count = ((lot_bottom - lot_top) / row_height).max(1);

    for r in 0..row_count {
        let y0 = lot_top + r * row_height;
        // Row separators.
        fill_rect(&mut pixels, 10, y0, (WIDTH as i32) - 20, 2, line_white);
        fill_rect(&mut pixels, 10, y0 + row_height - 2, (WIDTH as i32) - 20, 2, line_white);

        // Center lane marker (dashed yellow).
        let lane_y = y0 + (row_height / 2) - 1;
        let mut dx = 12;
        while dx < (WIDTH as i32) - 12 {
            fill_rect(&mut pixels, dx, lane_y, 8, 2, line_yellow);
            dx += 18;
        }

        // Parking spot vertical lines.
        let mut sx = 12;
        while sx < (WIDTH as i32) - 12 {
            fill_rect(&mut pixels, sx, y0 + 2, 2, 18, line_white);
            fill_rect(&mut pixels, sx, y0 + row_height - 20, 2, 18, line_white);
            sx += spot_w;
        }
    }

    // A few cars.
    draw_car(&mut pixels, 28, lot_top + 6, rgba(200, 64, 56));
    draw_car(&mut pixels, 64, lot_top + row_height - 26, rgba(72, 128, 210));
    draw_car(&mut pixels, 200, lot_top + row_height + 6, rgba(232, 232, 232));
    draw_car(&mut pixels, 176, lot_top + (2 * row_height) - 26, rgba(88, 206, 132));

    pixels
}

#[unsafe(no_mangle)]
pub extern "C" fn bvb_level_init() {
    // Idempotent init.
    if unsafe { !PIXELS_PTR.is_null() } {
        return;
    }

    let mut pixels = generate();
    let ptr = pixels.as_mut_ptr();
    let len = pixels.len();

    unsafe {
        PIXELS_PTR = ptr;
        PIXELS_LEN = len;
    }

    // Intentionally leak to keep the buffer alive for the lifetime of the module.
    core::mem::forget(pixels);
}

#[unsafe(no_mangle)]
pub extern "C" fn bvb_level_width() -> u32 {
    WIDTH
}

#[unsafe(no_mangle)]
pub extern "C" fn bvb_level_height() -> u32 {
    HEIGHT
}

#[unsafe(no_mangle)]
pub extern "C" fn bvb_level_pixels_ptr() -> *const u8 {
    unsafe { PIXELS_PTR as *const u8 }
}

#[unsafe(no_mangle)]
pub extern "C" fn bvb_level_pixels_len() -> u32 {
    unsafe { PIXELS_LEN as u32 }
}
