use anyhow::{anyhow, Result};
use wasm_bindgen::JsValue;
use web_sys::HtmlElement;

pub fn draw_ui(_html: &str) -> Result<()> {
    Ok(())
}

pub fn hide_ui() -> Result<()> {
    Ok(())
}

pub fn find_html_element_by_id(_id: &str) -> Result<HtmlElement> {
    Err(anyhow!("Not implemented yet!"))
}

pub async fn fetch_json(_json_path: &str) -> Result<JsValue> {
    Err(anyhow!("Not implemented yet!"))
}
