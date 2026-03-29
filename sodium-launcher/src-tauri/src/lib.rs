use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use url::form_urlencoded;
// ── Écoute le callback OAuth et retourne le code ─────────────────────────────

#[tauri::command]
async fn start_microsoft_auth() -> Result<String, String> {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();

    thread::spawn(move || {
        let listener = match TcpListener::bind("127.0.0.1:8080") {
            Ok(l) => l,
            Err(e) => {
                let _ = tx.send(Err(format!("Impossible de démarrer le serveur : {}", e)));
                return;
            }
        };

        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(Err(format!("Erreur d'acceptation : {}", e)));
                return;
            }
        };

        let mut buffer = [0; 8192];
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(Err(format!("Erreur de lecture : {}", e)));
                return;
            }
        };

        let request = String::from_utf8_lossy(&buffer[..n]);
        eprintln!("[Rust] Requête reçue : {}", request.lines().next().unwrap_or(""));

        let code = if let Some(pos) = request.find("code=") {
            let rest = &request[pos + 5..];
            let raw = rest
                .split(['&', ' ', '\r', '\n'])
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            raw.and_then(|c| {
                urlencoding::decode(&c).map(|d| d.into_owned()).ok()
            })
        } else {
            None
        };

        let body = r#"<html><body style='font-family:sans-serif;text-align:center;padding:80px;'>
            <h1 style='color:#107c10;'>✅ Connexion réussie !</h1>
            <p>Vous pouvez fermer cet onglet et retourner sur Sodium Launcher.</p>
            <script>window.close();</script>
        </body></html>"#;

        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();

        match code {
            Some(c) => {
                eprintln!("[Rust] Code extrait (fin) : ...{}", &c[c.len().saturating_sub(20)..]);
                let _ = tx.send(Ok(c));
            }
            None => {
                let _ = tx.send(Err("Code non trouvé dans la requête".to_string()));
            }
        }
    });

    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(result) => result,
        Err(_) => Err("Timeout : aucune réponse du navigateur".to_string()),
    }
}

// ── Échange le code contre un token Microsoft (sans header Origin) ────────────

#[tauri::command]
async fn exchange_microsoft_token(
    code: String,
    code_verifier: String,
    client_id: String,
    redirect_uri: String,
) -> Result<String, String> {
    let mut params = HashMap::new();
    params.insert("client_id",     client_id.as_str());
    params.insert("redirect_uri",  redirect_uri.as_str());
    params.insert("grant_type",    "authorization_code");
    params.insert("code",          code.as_str());
    params.insert("code_verifier", code_verifier.as_str());
    params.insert("scope",         "XboxLive.signin offline_access openid profile");

let body = form_urlencoded::Serializer::new(String::new())
    .append_pair("client_id",     &client_id)
    .append_pair("redirect_uri",  &redirect_uri)
    .append_pair("grant_type",    "authorization_code")
    .append_pair("code",          &code)
    .append_pair("code_verifier", &code_verifier)
    .append_pair("scope", "XboxLive.signin openid offline_access")

     .finish();

    let client = reqwest::Client::new();
    let res = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        // ✅ Pas de header Origin — c'est la clé
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Requête échouée : {}", e))?;

    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        eprintln!("[Rust] Token error: {}", text);
        return Err(format!("Token error {}: {}", status, text));
    }

    eprintln!("[Rust] Token obtenu avec succès");
    Ok(text) // JSON brut → parsé côté TypeScript
}

#[tauri::command]
async fn refresh_microsoft_token(
    refresh_token: String,
    client_id: String,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let params = [
        ("client_id", client_id.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("scope", "XboxLive.signin openid offline_access"),
    ];

    let res = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body = res.text().await.map_err(|e| e.to_string())?;
    Ok(body)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            start_microsoft_auth,
            exchange_microsoft_token,
            refresh_microsoft_token
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}