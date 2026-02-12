use js_sys::{Function, Promise, Reflect};
use wasm_bindgen::{JsCast, closure::Closure, prelude::*};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use workbox_rs::routing::{Method, register_route, set_default_handler};
use workbox_rs::strategies::{CacheFirst, HandlerFn};
use workbox_rs::sw;
use workbox_rs::types::{Request, Response};

const CACHE_STATIC: &str = "bvb-static-v3";
const OLD_CACHES: [&str; 2] = ["bvb-static-v1", "bvb-static-v2"];
const PRECACHE_URLS: [&str; 10] = [
    "/",
    "/index.html",
    "/beyond_vs_below.js",
    "/beyond_vs_below_bg.wasm",
    "/assets/title_screen/title_screen.png",
    "/assets/levels/mall_parking_lot.wasm?v=unit11",
    "/sw_bootstrap_sync.js",
    "/sw_bootstrap.js",
    "/assets/sw/bvb_sw.js",
    "/assets/sw/bvb_sw_bg.wasm",
];

#[wasm_bindgen]
pub fn start() {
    install_precache_listener();
    install_activate_listener();

    register_route(
        |request: &Request| should_cache_runtime(request),
        CacheFirst::new().cache_name(CACHE_STATIC),
        Some(Method::GET),
    );

    // Fallback for non-routed requests: cache hit first for GET, then network.
    set_default_handler(HandlerFn::new(|request: Request| async move {
        if request.method() == "GET" {
            if let Some(cached) = cache_match(&request).await? {
                return Ok(cached);
            }
        }

        let fetched = JsFuture::from(global_fetch(&request)?)
            .await
            .map_err(workbox_rs::WorkboxError::from_js_value)?;
        fetched
            .dyn_into::<Response>()
            .map_err(workbox_rs::WorkboxError::from_js_value)
    }));

    sw::start();
}

fn service_worker_scope() -> Result<web_sys::ServiceWorkerGlobalScope, workbox_rs::WorkboxError> {
    js_sys::global()
        .dyn_into::<web_sys::ServiceWorkerGlobalScope>()
        .map_err(|err| workbox_rs::WorkboxError::Js(format!("{err:?}")))
}

fn global_fetch(request: &Request) -> Result<js_sys::Promise, workbox_rs::WorkboxError> {
    let scope = service_worker_scope()?;
    Ok(scope.fetch_with_request(request))
}

fn should_cache_runtime(request: &Request) -> bool {
    if request.method() != "GET" {
        return false;
    }

    let Ok(url) = web_sys::Url::new(&request.url()) else {
        return false;
    };
    let Ok(scope) = service_worker_scope() else {
        return false;
    };
    let Ok(worker_scope) = scope.clone().dyn_into::<web_sys::WorkerGlobalScope>() else {
        return false;
    };
    let origin = worker_scope.location().origin();

    if url.origin() != origin {
        return false;
    }

    let host = url.hostname();
    if host == "127.0.0.1" || host == "localhost" {
        // Keep localhost predictable during Trunk live iteration; precache still
        // covers explicit shell assets for offline validation.
        return false;
    }

    let path = url.pathname();
    if path == "/sw_bootstrap.js" || path == "/sw_bootstrap_sync.js" || path.starts_with("/sw/") {
        return false;
    }

    true
}

async fn cache_match(request: &Request) -> Result<Option<Response>, workbox_rs::WorkboxError> {
    let scope = service_worker_scope()?;
    let cache_storage = scope
        .caches()
        .map_err(workbox_rs::WorkboxError::from_js_value)?;
    let cache_value = JsFuture::from(cache_storage.open(CACHE_STATIC))
        .await
        .map_err(workbox_rs::WorkboxError::from_js_value)?;
    let cache = cache_value
        .dyn_into::<web_sys::Cache>()
        .map_err(workbox_rs::WorkboxError::from_js_value)?;

    let matched = JsFuture::from(cache.match_with_request(request))
        .await
        .map_err(workbox_rs::WorkboxError::from_js_value)?;

    if matched.is_undefined() || matched.is_null() {
        return Ok(None);
    }

    Ok(Some(
        matched
            .dyn_into::<Response>()
            .map_err(workbox_rs::WorkboxError::from_js_value)?,
    ))
}

fn install_precache_listener() {
    let Ok(scope) = service_worker_scope() else {
        return;
    };

    let on_install = Closure::wrap(Box::new(move |event: web_sys::ExtendableEvent| {
        let promise = future_to_promise(async move {
            if let Err(err) = precache_assets().await {
                web_sys::console::error_1(&err);
            }
            if let Err(err) = skip_waiting() {
                web_sys::console::error_1(&err);
            }
            Ok(JsValue::UNDEFINED)
        });
        let _ = event.wait_until(&promise);
    }) as Box<dyn FnMut(_)>);

    let _ = scope.add_event_listener_with_callback("install", on_install.as_ref().unchecked_ref());
    on_install.forget();
}

fn install_activate_listener() {
    let Ok(scope) = service_worker_scope() else {
        return;
    };

    let on_activate = Closure::wrap(Box::new(move |event: web_sys::ExtendableEvent| {
        let promise = future_to_promise(async move {
            if let Err(err) = purge_old_caches().await {
                web_sys::console::error_1(&err);
            }
            if let Err(err) = claim_clients().await {
                web_sys::console::error_1(&err);
            }
            Ok(JsValue::UNDEFINED)
        });
        let _ = event.wait_until(&promise);
    }) as Box<dyn FnMut(_)>);

    let _ = scope.add_event_listener_with_callback("activate", on_activate.as_ref().unchecked_ref());
    on_activate.forget();
}

fn skip_waiting() -> Result<(), JsValue> {
    let scope = js_sys::global().dyn_into::<web_sys::ServiceWorkerGlobalScope>()?;
    let skip_waiting = Reflect::get(&scope, &JsValue::from_str("skipWaiting"))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("skipWaiting missing"))?;
    let _ = skip_waiting.call0(&scope)?;
    Ok(())
}

async fn claim_clients() -> Result<(), JsValue> {
    let scope = js_sys::global().dyn_into::<web_sys::ServiceWorkerGlobalScope>()?;
    let clients = Reflect::get(&scope, &JsValue::from_str("clients"))?;
    let claim = Reflect::get(&clients, &JsValue::from_str("claim"))?
        .dyn_into::<Function>()
        .map_err(|_| JsValue::from_str("clients.claim missing"))?;
    let promise_val = claim.call0(&clients)?;
    let _ = JsFuture::from(Promise::from(promise_val)).await?;
    Ok(())
}

async fn purge_old_caches() -> Result<(), JsValue> {
    let scope = js_sys::global().dyn_into::<web_sys::ServiceWorkerGlobalScope>()?;
    let cache_storage = scope.caches()?;
    for name in OLD_CACHES {
        if name == CACHE_STATIC {
            continue;
        }
        let _ = JsFuture::from(cache_storage.delete(name)).await?;
    }
    Ok(())
}

async fn precache_assets() -> Result<(), JsValue> {
    let scope = js_sys::global().dyn_into::<web_sys::ServiceWorkerGlobalScope>()?;
    let cache_storage = scope.caches()?;
    let cache_value = JsFuture::from(cache_storage.open(CACHE_STATIC)).await?;
    let cache = cache_value.dyn_into::<web_sys::Cache>()?;

    for url in PRECACHE_URLS {
        let request = web_sys::Request::new_with_str(url)?;
        let response_value = JsFuture::from(scope.fetch_with_request(&request)).await?;
        let response: web_sys::Response = response_value.dyn_into()?;
        if response.ok() || response.status() == 0 {
            let response_for_cache = response.clone()?;
            let _ = JsFuture::from(cache.put_with_request(&request, &response_for_cache)).await;
        }
    }

    Ok(())
}
