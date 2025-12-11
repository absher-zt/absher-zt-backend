use std::net::{Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use actix_web::{get, post, web, App, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web::middleware::Logger;
use actix_ws::{CloseCode, Message};
use dashmap::{DashMap, Entry};
use futures_channel::oneshot;
use log::{Level, LevelFilter};
use serde::{Deserialize, Serialize};
use crate::req_code::RequestCode;

pub mod req_code;

pub fn to_json_str(ser: &impl Serialize) -> String {
    serde_json::to_string(ser).unwrap_or_else(|_| {
        panic!("unable to turn {} to a json", core::any::type_name_of_val(ser))
    })
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AutofillData {
    name: Option<[String; 2]>,
    email: Option<String>,
    phone_number: Option<String>,
    id: Option<String>,
    /// stored as a base64 image
    profile_picture: Option<String>,
    /// stored as a base64 image
    license: Option<String>,
    id_image: Option<String>,
}


#[derive(Copy, Clone, Deserialize, Serialize)]
pub struct RequestedAutofillFields {
    #[serde(default)]
    name: bool,
    #[serde(default)]
    email: bool,
    #[serde(default)]
    phone_number: bool,
    #[serde(default)]
    id: bool,
    #[serde(default)]
    profile_picture: bool,
    #[serde(default)]
    license: bool,
    #[serde(default)]
    id_image: bool,
}


struct PendingRequest {
    notify: oneshot::Sender<AutofillData>,
    data_requested: RequestedAutofillFields,
    expires_at: Instant,
}

static MAP: LazyLock<DashMap<RequestCode, PendingRequest>> = LazyLock::new(DashMap::new);

pub fn new_request(
    selected: RequestedAutofillFields
) -> (RequestCode, tokio::time::Timeout<oneshot::Receiver<AutofillData>>) {
    loop {
        let code = RequestCode::new_rand();
        match MAP.entry(code) {
            Entry::Occupied(_) => continue,
            Entry::Vacant(vacant) => {
                let (tx, rx) = oneshot::channel();
                let timeout = Instant::now()
                    // 3 minutes offically but also add undocumented non guarenteed 3s
                    // grace
                    .checked_add(Duration::from_secs(3 * 60 + 3))
                    .unwrap();

                vacant.insert(PendingRequest {
                    notify: tx,
                    data_requested: selected,
                    expires_at: timeout,
                });

                break (code, tokio::time::timeout_at(timeout.into(), rx))
            }
        }
    }
}


#[get("/listen")]
async fn listen(req: HttpRequest, body: web::Payload) -> actix_web::Result<impl Responder> {
    let (response, mut session, mut msg_stream) =
        actix_ws::handle(&req, body)?;

    actix_web::rt::spawn(async move {
        let Some(Ok(Message::Text(json))) = msg_stream.recv().await else {
            let _ = session.close(Some((CloseCode::Error, "expected JSON specification").into())).await;
            return;
        };

        let Ok(request) = serde_json::from_str(&json) else {
            let _ = session.close(Some((CloseCode::Error, "invalid data specification JSON").into())).await;
            return;
        };

        let (code, data_rcv) = new_request(request);

        let Ok(()) = session.text(code.as_str()).await else {
            // web socket closed
            return;
        };

        let close = match data_rcv.await {
            Ok(Ok(data)) => {
                let _ = session.text(to_json_str(&data)).await;
                session.close(None)
            }
            Ok(Err(_)) | Err(_) => {
                session.close(Some((CloseCode::Error, "request timed out").into()))
            }
        };

        let _ = close.await;
    });

    Ok(response)
}

#[post("/requests/{code}")]
async fn resolve(code: web::Path<RequestCode>, data: web::Json<AutofillData>) -> impl Responder {
    let code = code.into_inner();
    let entry = MAP.remove(&code)
        .map(|(_, data)| data)
        .filter(|data| Instant::now() <= data.expires_at);

    let Some(request) = entry else {
        return HttpResponse::NotFound()
    };

    if request.notify.send(data.into_inner()).is_err() {
        // it was aproved, but nobody is listening
        return HttpResponse::Accepted()
    }

    HttpResponse::Ok()
}


#[get("/requests/{code}")]
async fn fetch(code: web::Path<RequestCode>) -> impl Responder {
    let code = code.into_inner();
    let entry = MAP.get(&code)
        .filter(|data| Instant::now() <= data.expires_at)
        .map(|entry| entry.data_requested);

    let Some(request) = entry else {
        return HttpResponse::NotFound().finish()
    };

    HttpResponse::Ok().body(to_json_str(&request))
}


#[get("/")]
async fn index_page() -> impl Responder {
    "why are you on the index page of an API????"
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Info)
        .init();
    log::set_max_level(LevelFilter::Info);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    defer::defer(move || {
        let _ = shutdown_tx.send(());
    });


    let remove_expried = async move {
        let map = LazyLock::force(&MAP);
        loop {
            tokio::time::sleep(Duration::from_secs(360)).await;
            let now = Instant::now();
            map.retain(|_code, data| now < data.expires_at);
        }
    };

    tokio::spawn(async move {
        tokio::select! {
            _ = shutdown_rx => {},
            _ = remove_expried => {}
        }
    });

    let app_builder = || {
        App::new()
            .service(index_page)
            .service(listen)
            .service(resolve)
            .service(fetch)
            .wrap(Logger::default())
            .wrap(actix_web::middleware::Compress::default())
            .wrap(actix_cors::Cors::permissive())
    };

    let sock = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 80));

    log::info!("listening on {sock}");

    HttpServer::new(app_builder)
        .bind(sock)?
        .run()
        .await
}