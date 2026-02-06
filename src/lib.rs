use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("Ready to duel, {name}.")
}

#[wasm_bindgen(start)]
pub fn start() {}
