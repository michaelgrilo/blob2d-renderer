// This crate compiles to a standalone `.wasm` module that exports a pixel-art level map.
//
// It intentionally uses a simple C-style ABI so the main app can instantiate it with
// `WebAssembly.instantiate` and read pixels from this module's linear memory.

const WIDTH: u32 = 288; // 18 tiles * 16 px (9:16)
const HEIGHT: u32 = 512; // 32 tiles * 16 px
const CHANNELS: u32 = 4;

static mut PIXELS_PTR: *mut u8 = core::ptr::null_mut();
static mut PIXELS_LEN: usize = 0;

const ALIEN_BMP: &[u8] = include_bytes!("../../../assets/characters/alien_256.bmp");
const DEMON_BMP: &[u8] = include_bytes!("../../../assets/characters/demon_256.bmp");

struct Sprite {
    width: usize,
    height: usize,
    pixels: Vec<u8>, // RGBA
}

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

fn fill_circle(pixels: &mut [u8], cx: i32, cy: i32, radius: i32, c: [u8; 4]) {
    let r2 = radius * radius;
    for y in (cy - radius)..=(cy + radius) {
        let dy = y - cy;
        let dy2 = dy * dy;
        for x in (cx - radius)..=(cx + radius) {
            let dx = x - cx;
            if (dx * dx) + dy2 <= r2 {
                put(pixels, x, y, c);
            }
        }
    }
}

fn fill_ring(pixels: &mut [u8], cx: i32, cy: i32, outer: i32, inner: i32, c: [u8; 4]) {
    let ro2 = outer * outer;
    let ri2 = inner * inner;
    for y in (cy - outer)..=(cy + outer) {
        let dy = y - cy;
        let dy2 = dy * dy;
        for x in (cx - outer)..=(cx + outer) {
            let dx = x - cx;
            let d2 = (dx * dx) + dy2;
            if d2 <= ro2 && d2 >= ri2 {
                put(pixels, x, y, c);
            }
        }
    }
}

fn draw_left_arrow(pixels: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    fill_rect(pixels, x + 2, y + 2, 8, 2, c);
    put(pixels, x + 1, y + 1, c);
    put(pixels, x + 1, y + 2, c);
    put(pixels, x, y + 2, c);
    put(pixels, x + 1, y + 3, c);
}

fn draw_out_text(pixels: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    // O
    rect_outline(pixels, x, y, 5, 7, c);
    // U
    fill_rect(pixels, x + 7, y, 1, 6, c);
    fill_rect(pixels, x + 11, y, 1, 6, c);
    fill_rect(pixels, x + 7, y + 6, 5, 1, c);
    // T
    fill_rect(pixels, x + 14, y, 5, 1, c);
    fill_rect(pixels, x + 16, y, 1, 7, c);
}

fn draw_mall_mark(pixels: &mut [u8], x: i32, y: i32, c: [u8; 4]) {
    // Tiny block "M" to label mall spawn.
    fill_rect(pixels, x, y, 1, 7, c);
    fill_rect(pixels, x + 6, y, 1, 7, c);
    fill_rect(pixels, x + 1, y + 1, 1, 2, c);
    fill_rect(pixels, x + 2, y + 2, 1, 2, c);
    fill_rect(pixels, x + 4, y + 2, 1, 2, c);
    fill_rect(pixels, x + 5, y + 1, 1, 2, c);
}

fn draw_tree(
    pixels: &mut [u8],
    cx: i32,
    cy: i32,
    trunk: [u8; 4],
    canopy_dark: [u8; 4],
    canopy_light: [u8; 4],
) {
    fill_rect(pixels, cx - 1, cy + 2, 2, 3, trunk);
    fill_circle(pixels, cx, cy, 4, canopy_dark);
    fill_circle(pixels, cx - 2, cy + 1, 2, canopy_light);
    fill_circle(pixels, cx + 2, cy + 1, 2, canopy_light);
}

fn draw_alien_sigil(
    pixels: &mut [u8],
    cx: i32,
    cy: i32,
    outline: [u8; 4],
    fill: [u8; 4],
    eye: [u8; 4],
) {
    // Smaller alien-head glyph sized for compact spawn pads.
    fill_circle(pixels, cx, cy, 9, outline);
    fill_circle(pixels, cx, cy - 1, 6, fill);
    fill_rect(pixels, cx - 6, cy + 4, 12, 2, outline);
    fill_circle(pixels, cx - 3, cy - 1, 1, eye);
    fill_circle(pixels, cx + 3, cy - 1, 1, eye);
    fill_rect(pixels, cx - 1, cy + 1, 2, 1, eye);
}

fn draw_burrow_mouth(pixels: &mut [u8], cx: i32, cy: i32) {
    fill_rect(pixels, cx - 13, cy + 5, 26, 6, rgba(20, 4, 8));
    for i in 0..5 {
        let x = cx - 11 + (i * 5);
        let tooth_h = if i % 2 == 0 { 5 } else { 3 };
        fill_rect(pixels, x, cy + 6, 2, tooth_h, rgba(238, 210, 186));
    }
}

fn carve_opening(
    pixels: &mut [u8],
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    lane: [u8; 4],
    edge: [u8; 4],
) {
    fill_rect(pixels, x, y, w, h, lane);
    rect_outline(pixels, x, y, w, h, edge);
    let stripe_y = y + (h / 2) - 1;
    fill_rect(pixels, x + 2, stripe_y, w - 4, 2, edge);
}

#[inline]
fn color_dist_sq(r: u8, g: u8, b: u8, bg: [u8; 3]) -> u32 {
    let dr = (r as i32) - (bg[0] as i32);
    let dg = (g as i32) - (bg[1] as i32);
    let db = (b as i32) - (bg[2] as i32);
    ((dr * dr) + (dg * dg) + (db * db)) as u32
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

    let width = read_le_i32(bytes, 18)?;
    let height_raw = read_le_i32(bytes, 22)?;
    let planes = read_le_u16(bytes, 26)?;
    let bpp = read_le_u16(bytes, 28)?;
    let compression = read_le_u32(bytes, 30)?;
    if planes != 1 {
        return None;
    }

    let w = width.unsigned_abs() as usize;
    let h = height_raw.unsigned_abs() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let top_down = height_raw < 0;
    let mut out = vec![0u8; w * h * 4];

    match bpp {
        24 => {
            if compression != 0 {
                return None;
            }
            let row_stride = (w * 3).div_ceil(4) * 4;
            let payload_len = row_stride.checked_mul(h)?;
            let payload = bytes.get(data_offset..data_offset + payload_len)?;

            for y in 0..h {
                let src_y = if top_down { y } else { h - 1 - y };
                let row = &payload[(src_y * row_stride)..((src_y + 1) * row_stride)];
                for x in 0..w {
                    let si = x * 3;
                    let di = (y * w + x) * 4;
                    out[di] = row[si + 2];
                    out[di + 1] = row[si + 1];
                    out[di + 2] = row[si];
                    out[di + 3] = 255;
                }
            }
        }
        32 => {
            let row_stride = w * 4;
            let payload_len = row_stride.checked_mul(h)?;
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

            for y in 0..h {
                let src_y = if top_down { y } else { h - 1 - y };
                let row = &payload[(src_y * row_stride)..((src_y + 1) * row_stride)];
                for x in 0..w {
                    let si = x * 4;
                    let di = (y * w + x) * 4;
                    let px = u32::from_le_bytes([row[si], row[si + 1], row[si + 2], row[si + 3]]);
                    out[di] = extract_channel(px, rmask);
                    out[di + 1] = extract_channel(px, gmask);
                    out[di + 2] = extract_channel(px, bmask);
                    out[di + 3] = extract_channel(px, amask);
                }
            }
        }
        _ => return None,
    }

    Some(Sprite {
        width: w,
        height: h,
        pixels: out,
    })
}

fn key_background_transparent(sprite: &mut Sprite) {
    let w = sprite.width;
    let h = sprite.height;
    if w == 0 || h == 0 {
        return;
    }

    let mut border_samples: Vec<[u8; 3]> = Vec::new();
    let step_x = (w / 12).max(1);
    let step_y = (h / 12).max(1);

    for x in (0..w).step_by(step_x) {
        let top_i = (x * 4) as usize;
        let bot_i = (((h - 1) * w + x) * 4) as usize;
        border_samples.push([
            sprite.pixels[top_i],
            sprite.pixels[top_i + 1],
            sprite.pixels[top_i + 2],
        ]);
        border_samples.push([
            sprite.pixels[bot_i],
            sprite.pixels[bot_i + 1],
            sprite.pixels[bot_i + 2],
        ]);
    }
    for y in (0..h).step_by(step_y) {
        let left_i = ((y * w) * 4) as usize;
        let right_i = ((y * w + (w - 1)) * 4) as usize;
        border_samples.push([
            sprite.pixels[left_i],
            sprite.pixels[left_i + 1],
            sprite.pixels[left_i + 2],
        ]);
        border_samples.push([
            sprite.pixels[right_i],
            sprite.pixels[right_i + 1],
            sprite.pixels[right_i + 2],
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
    let count = border_samples.len() as u32;
    let bg = [
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
    ];

    let mut spread = 0u32;
    for s in &border_samples {
        spread = spread.max(color_dist_sq(s[0], s[1], s[2], bg));
    }
    let threshold = (spread + 900).clamp(900, 6400);

    for px in sprite.pixels.chunks_exact_mut(4) {
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
    let w = sprite.width;
    let h = sprite.height;
    let p = &sprite.pixels;
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

    let nw = max_x - min_x + 1;
    let nh = max_y - min_y + 1;
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        for x in 0..nw {
            let si = ((min_y + y) * w + (min_x + x)) * 4;
            let di = (y * nw + x) * 4;
            out[di..di + 4].copy_from_slice(&p[si..si + 4]);
        }
    }

    Some(Sprite {
        width: nw,
        height: nh,
        pixels: out,
    })
}

fn resize_nearest(sprite: &Sprite, target_h: usize) -> Sprite {
    let target_h = target_h.max(1);
    let target_w = ((sprite.width * target_h) / sprite.height).max(1);
    let mut out = vec![0u8; target_w * target_h * 4];

    for y in 0..target_h {
        let sy = (y * sprite.height) / target_h;
        for x in 0..target_w {
            let sx = (x * sprite.width) / target_w;
            let si = (sy * sprite.width + sx) * 4;
            let di = (y * target_w + x) * 4;
            out[di..di + 4].copy_from_slice(&sprite.pixels[si..si + 4]);
        }
    }

    Sprite {
        width: target_w,
        height: target_h,
        pixels: out,
    }
}

fn load_spawn_sprite(bytes: &[u8], target_h: usize) -> Option<Sprite> {
    let mut sprite = decode_bmp_to_rgba(bytes)?;
    key_background_transparent(&mut sprite);
    let cropped = crop_to_alpha(sprite)?;
    Some(resize_nearest(&cropped, target_h))
}

fn blend_pixel(dst: &mut [u8], src: &[u8]) {
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

fn blit_sprite_center_bottom(
    pixels: &mut [u8],
    sprite: &Sprite,
    center_x: i32,
    bottom_y: i32,
    x_offset: i32,
) {
    let start_x = center_x - (sprite.width as i32 / 2) + x_offset;
    let start_y = bottom_y - sprite.height as i32;

    for sy in 0..(sprite.height as i32) {
        let dy = start_y + sy;
        if !(0..(HEIGHT as i32)).contains(&dy) {
            continue;
        }
        for sx in 0..(sprite.width as i32) {
            let dx = start_x + sx;
            if !(0..(WIDTH as i32)).contains(&dx) {
                continue;
            }
            let si = ((sy as usize * sprite.width + sx as usize) * 4) as usize;
            if sprite.pixels[si + 3] == 0 {
                continue;
            }
            let di = (((dy as u32 * WIDTH + dx as u32) * CHANNELS) as usize) as usize;
            blend_pixel(&mut pixels[di..di + 4], &sprite.pixels[si..si + 4]);
        }
    }
}

fn generate() -> Vec<u8> {
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * CHANNELS) as usize];

    // Deliberately restrained palette for strong objective readability.
    let asphalt_a = rgba(14, 22, 30);
    let asphalt_b = rgba(18, 28, 38);
    let lane_tint = rgba(30, 46, 62);
    let lot_surface = rgba(24, 38, 52);
    let curb = rgba(88, 98, 108);
    let line_white = rgba(222, 230, 238);
    let line_yellow = rgba(226, 188, 72);
    let island_fill = rgba(84, 112, 72);
    let island_grass = rgba(104, 166, 88);
    let island_grass_dark = rgba(66, 122, 64);
    let island_tree = rgba(82, 152, 72);
    let island_tree_highlight = rgba(132, 196, 108);
    let island_trunk = rgba(102, 76, 48);
    let island_edge = rgba(196, 206, 154);

    let alien_outer = rgba(58, 226, 248);
    let alien_inner = rgba(26, 142, 196);
    let alien_core = rgba(214, 250, 255);

    let demon_outer = rgba(128, 36, 42);
    let demon_inner = rgba(62, 14, 18);
    let demon_core = rgba(210, 74, 40);

    let exit_road = rgba(38, 66, 58);
    let exit_glow = rgba(126, 216, 170);

    let mall_wall = rgba(84, 96, 112);
    let mall_shadow = rgba(58, 68, 82);
    let mall_door = rgba(18, 24, 34);
    let mall_walk = rgba(104, 142, 166);

    // Base asphalt with very light dithering.
    for y in 0..HEIGHT as i32 {
        for x in 0..WIDTH as i32 {
            let pick = ((x * 3 + y * 5) & 7) as i32;
            let c = if pick < 4 { asphalt_a } else { asphalt_b };
            put(&mut pixels, x, y, c);
        }
    }

    // Parking lot boundary curb.
    fill_rect(&mut pixels, 0, 0, WIDTH as i32, 8, curb);
    fill_rect(&mut pixels, 0, (HEIGHT as i32) - 8, WIDTH as i32, 8, curb);
    fill_rect(&mut pixels, 0, 0, 8, HEIGHT as i32, curb);
    fill_rect(&mut pixels, (WIDTH as i32) - 8, 0, 8, HEIGHT as i32, curb);

    // Symmetric compact base sizes for alien and demon spawn points.
    let base_outer_r = 30;
    let base_inner_r = 22;
    let base_core_r = 10;

    let alien_cx = (WIDTH as i32) / 2;
    let alien_cy = 66;
    let demon_cx = (WIDTH as i32) / 2;
    let demon_cy = (HEIGHT as i32) - 66;

    // Top and bottom spawn platforms.
    fill_rect(&mut pixels, 44, 34, (WIDTH as i32) - 88, 62, lane_tint);
    rect_outline(
        &mut pixels,
        44,
        34,
        (WIDTH as i32) - 88,
        62,
        rgba(120, 226, 248),
    );
    fill_rect(
        &mut pixels,
        44,
        (HEIGHT as i32) - 96,
        (WIDTH as i32) - 88,
        62,
        lane_tint,
    );
    rect_outline(
        &mut pixels,
        44,
        (HEIGHT as i32) - 96,
        (WIDTH as i32) - 88,
        62,
        rgba(220, 122, 78),
    );

    // Three MOBA-style lanes.
    let lane_centers = [86, (WIDTH as i32) / 2, (WIDTH as i32) - 86];
    let lane_w = 36;
    let lane_top = alien_cy + base_outer_r + 12;
    let lane_bottom = demon_cy - base_outer_r - 12;

    for lane_center in lane_centers {
        let x0 = lane_center - (lane_w / 2);
        fill_rect(
            &mut pixels,
            x0,
            lane_top,
            lane_w,
            lane_bottom - lane_top,
            lane_tint,
        );
        rect_outline(
            &mut pixels,
            x0,
            lane_top,
            lane_w,
            lane_bottom - lane_top,
            line_yellow,
        );

        let mut dash_y = lane_top + 12;
        while dash_y < lane_bottom - 12 {
            fill_rect(&mut pixels, lane_center - 1, dash_y, 2, 8, line_white);
            dash_y += 20;
        }

        // Connectors into both spawn circles.
        fill_rect(&mut pixels, x0 - 2, lane_top - 14, lane_w + 4, 14, lane_tint);
        fill_rect(&mut pixels, x0 - 2, lane_bottom, lane_w + 4, 14, lane_tint);
    }

    // Median islands between lanes, with staggered openings and a full middle crossover.
    let left_x0 = lane_centers[0] - (lane_w / 2);
    let center_x0 = lane_centers[1] - (lane_w / 2);
    let gap_lc_x = left_x0 + lane_w;
    let gap_cr_x = center_x0 + lane_w;
    let gap_w = center_x0 - gap_lc_x;
    let island_y = lane_top + 6;
    let island_h = lane_bottom - lane_top - 12;

    fill_rect(&mut pixels, gap_lc_x, island_y, gap_w, island_h, island_fill);
    rect_outline(&mut pixels, gap_lc_x, island_y, gap_w, island_h, island_edge);
    fill_rect(
        &mut pixels,
        gap_lc_x + 2,
        island_y + 2,
        gap_w - 4,
        island_h - 4,
        island_grass,
    );

    fill_rect(&mut pixels, gap_cr_x, island_y, gap_w, island_h, island_fill);
    rect_outline(&mut pixels, gap_cr_x, island_y, gap_w, island_h, island_edge);
    fill_rect(
        &mut pixels,
        gap_cr_x + 2,
        island_y + 2,
        gap_w - 4,
        island_h - 4,
        island_grass,
    );

    for y in ((island_y + 8)..(island_y + island_h - 8)).step_by(14usize) {
        fill_rect(&mut pixels, gap_lc_x + 3, y, gap_w - 6, 1, island_grass_dark);
        fill_rect(&mut pixels, gap_cr_x + 3, y, gap_w - 6, 1, island_grass_dark);
    }
    for y in ((island_y + 24)..(island_y + island_h - 22)).step_by(56usize) {
        draw_tree(
            &mut pixels,
            gap_lc_x + (gap_w / 2),
            y,
            island_trunk,
            island_tree,
            island_tree_highlight,
        );
        draw_tree(
            &mut pixels,
            gap_cr_x + (gap_w / 2),
            y,
            island_trunk,
            island_tree,
            island_tree_highlight,
        );
    }

    // Full middle opening: connects all three lanes.
    let mid_open_y = ((lane_top + lane_bottom) / 2) - 12;
    let mid_open_h = 24;
    carve_opening(
        &mut pixels,
        gap_lc_x,
        mid_open_y,
        gap_w,
        mid_open_h,
        lane_tint,
        line_white,
    );
    carve_opening(
        &mut pixels,
        gap_cr_x,
        mid_open_y,
        gap_w,
        mid_open_h,
        lane_tint,
        line_white,
    );

    // Staggered extra openings for rotational depth.
    let upper_open_y = lane_top + 58;
    let lower_open_y = lane_bottom - 78;
    let stagger_h = 20;
    carve_opening(
        &mut pixels,
        gap_lc_x,
        upper_open_y,
        gap_w,
        stagger_h,
        lane_tint,
        line_white,
    );
    carve_opening(
        &mut pixels,
        gap_cr_x,
        lower_open_y,
        gap_w,
        stagger_h,
        lane_tint,
        line_white,
    );

    // Parking bays.
    fill_rect(&mut pixels, 14, lane_top + 2, 38, lane_bottom - lane_top - 4, lot_surface);
    fill_rect(
        &mut pixels,
        (WIDTH as i32) - 52,
        lane_top + 2,
        38,
        lane_bottom - lane_top - 4,
        lot_surface,
    );
    let mut mark_y = lane_top + 14;
    while mark_y < lane_bottom - 10 {
        fill_rect(&mut pixels, 16, mark_y, 34, 1, line_white);
        fill_rect(&mut pixels, (WIDTH as i32) - 50, mark_y, 34, 1, line_white);
        mark_y += 26;
    }

    // Left-side parking lot exit (human win condition).
    let exit_y = ((lane_top + lane_bottom) / 2) - 18;
    let exit_h = 36;
    fill_rect(&mut pixels, 0, exit_y - 4, 8, exit_h + 8, asphalt_a); // curb break
    fill_rect(&mut pixels, 8, exit_y, 60, exit_h, exit_road);
    rect_outline(&mut pixels, 8, exit_y, 60, exit_h, exit_glow);
    fill_rect(&mut pixels, 12, exit_y + (exit_h / 2) - 1, 46, 2, line_yellow);
    fill_rect(&mut pixels, 55, exit_y + 6, 3, exit_h - 12, exit_glow);
    draw_left_arrow(&mut pixels, 18, exit_y + 7, line_white);
    draw_left_arrow(&mut pixels, 18, exit_y + 19, line_white);

    // "OUT" sign so the left opening reads as a clear parking-lot exit.
    fill_rect(&mut pixels, 12, exit_y - 22, 34, 14, exit_glow);
    fill_rect(&mut pixels, 13, exit_y - 21, 32, 12, rgba(8, 20, 16));
    draw_out_text(&mut pixels, 17, exit_y - 18, line_white);
    fill_rect(&mut pixels, 28, exit_y - 8, 2, 8, curb);

    // Right-side mall entrance (human spawn point).
    let entrance_w = 48;
    let entrance_h = 56;
    let entrance_x = (WIDTH as i32) - 8 - entrance_w;
    let entrance_y = ((lane_top + lane_bottom) / 2) - 28;
    fill_rect(
        &mut pixels,
        entrance_x,
        entrance_y,
        entrance_w,
        entrance_h,
        mall_wall,
    );
    rect_outline(
        &mut pixels,
        entrance_x,
        entrance_y,
        entrance_w,
        entrance_h,
        rgba(172, 196, 214),
    );
    fill_rect(
        &mut pixels,
        entrance_x + 4,
        entrance_y + 4,
        entrance_w - 8,
        8,
        mall_shadow,
    );
    fill_rect(
        &mut pixels,
        entrance_x + 16,
        entrance_y + 22,
        16,
        24,
        mall_door,
    );
    rect_outline(
        &mut pixels,
        entrance_x + 16,
        entrance_y + 22,
        16,
        24,
        rgba(118, 146, 170),
    );
    fill_rect(
        &mut pixels,
        entrance_x + 22,
        entrance_y + 22,
        1,
        24,
        rgba(118, 146, 170),
    );
    fill_rect(
        &mut pixels,
        entrance_x + 25,
        entrance_y + 22,
        1,
        24,
        rgba(118, 146, 170),
    );
    fill_rect(
        &mut pixels,
        entrance_x - 12,
        entrance_y + 30,
        12,
        8,
        mall_walk,
    );
    rect_outline(
        &mut pixels,
        entrance_x - 12,
        entrance_y + 30,
        12,
        8,
        rgba(176, 204, 220),
    );
    fill_rect(
        &mut pixels,
        entrance_x + 13,
        entrance_y + 12,
        22,
        8,
        mall_walk,
    );
    rect_outline(
        &mut pixels,
        entrance_x + 13,
        entrance_y + 12,
        22,
        8,
        rgba(176, 204, 220),
    );
    draw_mall_mark(
        &mut pixels,
        entrance_x + 20,
        entrance_y + 13,
        rgba(30, 52, 70),
    );

    // Alien spawn base (top).
    fill_circle(&mut pixels, alien_cx, alien_cy, base_outer_r, alien_outer);
    fill_circle(&mut pixels, alien_cx, alien_cy, base_inner_r, alien_inner);
    fill_ring(
        &mut pixels,
        alien_cx,
        alien_cy,
        base_outer_r,
        base_outer_r - 4,
        line_white,
    );
    fill_ring(&mut pixels, alien_cx, alien_cy, 14, 10, rgba(88, 228, 255));
    fill_circle(&mut pixels, alien_cx, alien_cy, base_core_r, alien_core);
    draw_alien_sigil(
        &mut pixels,
        alien_cx,
        alien_cy,
        rgba(20, 34, 54),
        rgba(112, 240, 255),
        rgba(8, 16, 30),
    );

    // Demon spawn base (bottom).
    fill_circle(&mut pixels, demon_cx, demon_cy, base_outer_r, demon_outer);
    fill_circle(&mut pixels, demon_cx, demon_cy, base_inner_r, demon_inner);
    fill_ring(
        &mut pixels,
        demon_cx,
        demon_cy,
        base_outer_r,
        base_outer_r - 4,
        rgba(226, 120, 68),
    );
    fill_circle(&mut pixels, demon_cx, demon_cy, base_core_r, demon_core);
    draw_burrow_mouth(&mut pixels, demon_cx, demon_cy);

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
