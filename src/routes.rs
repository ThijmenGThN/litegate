use crate::{
    db::{Db, Payment},
    electrum::rpc_async,
    utils::{encrypt_wif, new_key, script_hash},
};
use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::json;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::task::spawn_blocking;
use uuid::Uuid;

#[derive(Deserialize, Debug)]
struct PayReq {
    amount: f64,
    ttl: u64,
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/health").route(web::get().to(health_check)))
        .service(web::resource("/payments").route(web::post().to(create_payment)))
        .service(web::resource("/payments/{id}").route(web::get().to(get_payment)));
}

async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(json!({
        "status": "ok",
        "timestamp": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }))
}

async fn create_payment(db: web::Data<Db>, req: web::Json<PayReq>) -> HttpResponse {
    if req.amount <= 0.0 {
        return HttpResponse::BadRequest().finish();
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let expires_at = if req.ttl == 0 { 0 } else { now + req.ttl as i64 };
    let id = Uuid::new_v4().to_string();
    let (_, wif, addr) = new_key();
    let wif_enc = encrypt_wif(&wif);
    let payment = Payment {
        id: id.clone(),
        address: addr.clone(),
        wif_enc,
        amount: req.amount,
        status: "pending".into(),
        created_at: 0,
        updated_at: 0,
        expires_at,
    };
    let db_clone = db.clone();
    let payment_clone = payment.clone();
    if spawn_blocking(move || db_clone.insert(&payment_clone))
        .await
        .unwrap()
        .is_err()
    {
        return HttpResponse::InternalServerError().finish();
    }
    HttpResponse::Ok().json(json!({
        "id": id,
        "address": addr,
        "amount": req.amount,
        "expires_at": expires_at
    }))
}

async fn get_payment(db: web::Data<Db>, path: web::Path<String>) -> HttpResponse {
    let payment_id = path.into_inner();
    let db_clone = db.clone();
    let payment_opt = spawn_blocking(move || db_clone.find(&payment_id))
        .await
        .unwrap()
        .unwrap_or(None);
    let Some(mut payment) = payment_opt else {
        return HttpResponse::NotFound().finish();
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    if payment.status == "pending" && payment.expires_at != 0 && payment.expires_at < now {
        let _ = db.mark_expired(&payment.id);
        payment.status = "expired".into();
    }
    let bal = match rpc_async(
        "blockchain.scripthash.get_balance",
        &[script_hash(&payment.address).into()],
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return HttpResponse::BadGateway().finish(),
    };
    let hdr = match rpc_async("blockchain.headers.subscribe", &[]).await {
        Ok(v) => v,
        Err(_) => return HttpResponse::BadGateway().finish(),
    };
    let hist = match rpc_async(
        "blockchain.scripthash.get_history",
        &[script_hash(&payment.address).into()],
    )
    .await
    {
        Ok(v) => v,
        Err(_) => return HttpResponse::BadGateway().finish(),
    };
    let tip = hdr["height"].as_u64().unwrap_or(0);
    let confirmations = hist
        .as_array()
        .unwrap()
        .iter()
        .filter(|h| h["height"].as_u64().unwrap_or(0) > 0)
        .map(|h| tip - h["height"].as_u64().unwrap() + 1)
        .min()
        .unwrap_or(0);
    let confirmed_sat = bal["confirmed"].as_i64().unwrap_or(0).max(0) as f64;
    let unconfirmed_sat = bal["unconfirmed"].as_i64().unwrap_or(0).max(0) as f64;
    let received = (confirmed_sat + unconfirmed_sat) / 1e8;
    HttpResponse::Ok().json(json!({
        "id": payment.id,
        "address": payment.address,
        "amount": payment.amount,
        "status": payment.status,
        "created_at": payment.created_at,
        "updated_at": payment.updated_at,
        "expires_at": payment.expires_at,
        "confirmations": confirmations,
        "received": received,
    }))
}
