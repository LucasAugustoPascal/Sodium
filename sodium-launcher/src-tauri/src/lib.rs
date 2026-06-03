
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::process::Command;
use url::form_urlencoded;
use dirs;
use serde::Deserialize;
use tauri::Emitter;

#[derive(Deserialize)]
struct LaunchOptions {
    version: String,
    access_token: String,
    uuid: String,
    name: String,
}

fn get_game_dir() -> Result<std::path::PathBuf, String> {
    let base = dirs::home_dir().ok_or("Home introuvable")?;
    #[cfg(target_os = "macos")]
    {
        Ok(base.join("Library").join("Application Support").join("sodium"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(base.join(".sodium"))
    }
}

#[tauri::command]
async fn install_version(window: tauri::Window, version: String) -> Result<(), String> {
    let game_dir = get_game_dir()?;

    let version_dir = game_dir.join("versions").join(&version);
    std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;

    let jar_path   = version_dir.join(format!("{}.jar", version));
    let json_path  = version_dir.join(format!("{}.json", version));
    let indexes_dir = game_dir.join("assets").join("indexes");

    let _ = window.emit("install_progress", serde_json::json!({
        "stage": "manifest", "percent": 0, "label": "Récupération du manifest..."
    }));

    let client = reqwest::Client::new();

    let manifest: serde_json::Value = client
        .get("https://launchermeta.mojang.com/mc/game/version_manifest.json")
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    let version_url = manifest["versions"]
        .as_array().ok_or("Manifest invalide")?
        .iter()
        .find(|v| v["id"].as_str() == Some(&version))
        .ok_or(format!("Version {} introuvable", version))?["url"]
        .as_str().ok_or("URL invalide")?
        .to_string();

    let version_json: serde_json::Value = client
        .get(&version_url)
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    std::fs::write(&json_path, serde_json::to_string_pretty(&version_json).unwrap())
        .map_err(|e| e.to_string())?;

    let asset_index     = &version_json["assetIndex"];
    let asset_index_id  = asset_index["id"].as_str().ok_or("assetIndex id introuvable")?.to_string();
    let asset_index_url = asset_index["url"].as_str().ok_or("assetIndex url introuvable")?.to_string();
    let index_path      = indexes_dir.join(format!("{}.json", asset_index_id));

    if jar_path.exists() && json_path.exists() && index_path.exists() {
        let _ = window.emit("install_progress", serde_json::json!({
            "stage": "done", "percent": 100, "label": "Déjà installé ✅"
        }));
        eprintln!("[Install] Version {} déjà installée", version);
        return Ok(());
    }

    let _ = window.emit("install_progress", serde_json::json!({
        "stage": "jar", "percent": 5, "label": "Téléchargement du client Minecraft..."
    }));

    let jar_url = version_json["downloads"]["client"]["url"]
        .as_str().ok_or("URL JAR introuvable")?.to_string();

    let jar_bytes = client.get(&jar_url).send().await.map_err(|e| e.to_string())?
        .bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(&jar_path, &jar_bytes).map_err(|e| e.to_string())?;

    let libs_dir  = game_dir.join("libraries");
    let libs      = version_json["libraries"].as_array().ok_or("libraries introuvable")?;
    let lib_total = libs.len();

    for (i, lib) in libs.iter().enumerate() {
        if let Some(artifact) = lib["downloads"]["artifact"].as_object() {
            let path = artifact["path"].as_str().unwrap_or("");
            let url  = artifact["url"].as_str().unwrap_or("");
            if path.is_empty() || url.is_empty() { continue; }

            let lib_path = libs_dir.join(path);
            if lib_path.exists() { continue; }

            if let Some(parent) = lib_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let bytes = client.get(url).send().await.map_err(|e| e.to_string())?
                .bytes().await.map_err(|e| e.to_string())?;
            std::fs::write(&lib_path, &bytes).map_err(|e| e.to_string())?;
        }

        let percent = 10 + (i * 20 / lib_total.max(1));
        let _ = window.emit("install_progress", serde_json::json!({
            "stage": "libraries",
            "percent": percent,
            "label": format!("Libraries {}/{}", i + 1, lib_total)
        }));
    }

    let _ = window.emit("install_progress", serde_json::json!({
        "stage": "asset_index", "percent": 30, "label": "Téléchargement de l'index des assets..."
    }));

    std::fs::create_dir_all(&indexes_dir).map_err(|e| e.to_string())?;

    let asset_index_json: serde_json::Value = client
        .get(&asset_index_url)
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    std::fs::write(&index_path, serde_json::to_string_pretty(&asset_index_json).unwrap())
        .map_err(|e| e.to_string())?;

    let objects_dir = game_dir.join("assets").join("objects");

    if let Some(objects) = asset_index_json["objects"].as_object() {
        let total = objects.len();
        let mut count = 0usize;

        for (_name, obj) in objects {
            let hash = obj["hash"].as_str().unwrap_or("");
            if hash.len() < 2 { continue; }

            let prefix   = &hash[..2];
            let obj_dir  = objects_dir.join(prefix);
            std::fs::create_dir_all(&obj_dir).map_err(|e| e.to_string())?;

            let obj_path = obj_dir.join(hash);
            if !obj_path.exists() {
                let url   = format!("https://resources.download.minecraft.net/{}/{}", prefix, hash);
                let bytes = client.get(&url).send().await.map_err(|e| e.to_string())?
                    .bytes().await.map_err(|e| e.to_string())?;
                std::fs::write(&obj_path, &bytes).map_err(|e| e.to_string())?;
            }

            count += 1;
            if count % 50 == 0 || count == total {
                let percent = 30 + (count * 70 / total.max(1));
                let _ = window.emit("install_progress", serde_json::json!({
                    "stage": "assets",
                    "percent": percent,
                    "label": format!("Assets {}/{}", count, total)
                }));
            }
        }
    }

    let _ = window.emit("install_progress", serde_json::json!({
        "stage": "done", "percent": 100, "label": "Installation terminée ✅"
    }));

    eprintln!("[Install] ✅ Version {} installée avec assets", version);
    Ok(())
}

#[tauri::command]
async fn install_fabric(minecraft_version: String) -> Result<String, String> {
    let game_dir = get_game_dir()?;
    let client   = reqwest::Client::new();

    let loaders: serde_json::Value = client
        .get("https://meta.fabricmc.net/v2/versions/loader")
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    let loader_version = loaders.as_array()
        .and_then(|a| a.iter().find(|v| v["stable"].as_bool() == Some(true)))
        .and_then(|v| v["version"].as_str())
        .ok_or("Loader Fabric introuvable")?
        .to_string();

    eprintln!("[Fabric] Loader: {}", loader_version);

    let fabric_version_id = format!("fabric-loader-{}-{}", loader_version, minecraft_version);
    let fabric_json_url   = format!(
        "https://meta.fabricmc.net/v2/versions/loader/{}/{}/profile/json",
        minecraft_version, loader_version
    );

    let response = client.get(&fabric_json_url).send().await.map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        let body = response.text().await.map_err(|e| e.to_string())?;
        return Err(format!("Fabric JSON error: {}", body));
    }

    let fabric_json: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;

    let version_dir = game_dir.join("versions").join(&fabric_version_id);
    std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;

    let libs_dir  = game_dir.join("libraries");
    let json_path = version_dir.join(format!("{}.json", fabric_version_id));
    std::fs::write(&json_path, serde_json::to_string_pretty(&fabric_json).unwrap())
        .map_err(|e| e.to_string())?;

    if let Some(libs) = fabric_json["libraries"].as_array() {
        for lib in libs {
            let name = lib["name"].as_str().unwrap_or("");
            let url  = lib["url"].as_str().unwrap_or("https://maven.fabricmc.net/");

            let parts: Vec<&str> = name.splitn(3, ':').collect();
            if parts.len() != 3 { continue; }

            let group_path = parts[0].replace('.', "/");
            let artifact   = parts[1];
            let version    = parts[2];
            let jar_name   = format!("{}-{}.jar", artifact, version);
            let lib_path   = format!("{}/{}/{}/{}", group_path, artifact, version, jar_name);
            let full_path  = libs_dir.join(&lib_path);

            if full_path.exists() { continue; }

            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }

            let download_url = format!("{}/{}", url.trim_end_matches('/'), lib_path);
            let res = client.get(&download_url).send().await.map_err(|e| e.to_string())?;

            if !res.status().is_success() {
                eprintln!("[Fabric] ⚠️ Skip {} (status {})", download_url, res.status());
                continue;
            }

            let bytes = res.bytes().await.map_err(|e| e.to_string())?;
            std::fs::write(&full_path, &bytes).map_err(|e| e.to_string())?;
            eprintln!("[Fabric] ✅ {}", jar_name);
        }
    }

    eprintln!("[Fabric] ✅ Installation terminée : {}", fabric_version_id);
    Ok(fabric_version_id)
}

#[tauri::command]
async fn launch_minecraft(args: LaunchOptions) -> Result<(), String> {
    let version      = args.version;
    let access_token = args.access_token;
    let uuid         = args.uuid;
    let name         = args.name;

    let game_dir         = get_game_dir()?;
    let versions_dir     = game_dir.join("versions").join(&version);
    let libs_dir         = game_dir.join("libraries");
    let version_json_path = versions_dir.join(format!("{}.json", version));

    let content = std::fs::read_to_string(&version_json_path)
        .map_err(|e| format!("Erreur lecture JSON : {}", e))?;
    let raw: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Erreur parsing JSON : {}", e))?;

    let main_class = raw["mainClass"].as_str().ok_or("mainClass introuvable")?.to_string();
    eprintln!("[Launch] mainClass: {}", main_class);

    // Asset index id : depuis la version vanilla si inheritsFrom
    let asset_index_id = if let Some(inherits) = raw["inheritsFrom"].as_str() {
        let vanilla_path = game_dir
            .join("versions").join(inherits)
            .join(format!("{}.json", inherits));
        let vanilla_raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&vanilla_path)
                .map_err(|e| format!("Erreur lecture JSON vanilla pour assets : {}", e))?
        ).map_err(|e| format!("Erreur parsing JSON vanilla pour assets : {}", e))?;
        vanilla_raw["assetIndex"]["id"].as_str().unwrap_or("17").to_string()
    } else {
        raw["assetIndex"]["id"].as_str().unwrap_or("17").to_string()
    };

    eprintln!("[Launch] assetIndex: {}", asset_index_id);

    let mut all_libraries: Vec<serde_json::Value> = raw["libraries"]
        .as_array().ok_or("libraries introuvable")?.clone();

    if let Some(inherits) = raw["inheritsFrom"].as_str() {
        let vanilla_path = game_dir
            .join("versions").join(inherits)
            .join(format!("{}.json", inherits));
        let vanilla_raw: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&vanilla_path)
                .map_err(|e| format!("Erreur lecture JSON vanilla : {}", e))?
        ).map_err(|e| format!("Erreur parsing JSON vanilla : {}", e))?;

        if let Some(vanilla_libs) = vanilla_raw["libraries"].as_array() {
            eprintln!("[Launch] Ajout de {} libraries vanilla", vanilla_libs.len());
            all_libraries.extend(vanilla_libs.clone());
        }
    }

    let mut cp_parts = Vec::new();

    if let Some(inherits) = raw["inheritsFrom"].as_str() {
        let vanilla_jar = game_dir
            .join("versions").join(inherits)
            .join(format!("{}.jar", inherits));
        eprintln!("[Launch] JAR: {}", vanilla_jar.display());
        cp_parts.push(vanilla_jar.to_string_lossy().into_owned());
    } else {
        let jar = versions_dir.join(format!("{}.jar", version));
        eprintln!("[Launch] JAR: {}", jar.display());
        cp_parts.push(jar.to_string_lossy().into_owned());
    }

    for lib in &all_libraries {
        if let Some(path) = lib["downloads"]["artifact"]["path"].as_str() {
            let full = libs_dir.join(path);
            if full.exists() {
                cp_parts.push(full.to_string_lossy().into_owned());
            } else {
                eprintln!("[Launch] ⚠️ Manquante (vanilla): {}", path);
            }
            continue;
        }
        if let Some(lib_name) = lib["name"].as_str() {
            let parts: Vec<&str> = lib_name.splitn(3, ':').collect();
            if parts.len() == 3 {
                let group_path = parts[0].replace('.', "/");
                let artifact   = parts[1];
                let ver        = parts[2];
                let jar_name   = format!("{}-{}.jar", artifact, ver);
                let lib_path   = format!("{}/{}/{}/{}", group_path, artifact, ver, jar_name);
                let full       = libs_dir.join(&lib_path);
                if full.exists() {
                    cp_parts.push(full.to_string_lossy().into_owned());
                } else {
                    eprintln!("[Launch] ⚠️ Manquante (fabric): {}", lib_path);
                }
            }
        }
    }

    eprintln!("[Launch] {} entries dans le classpath", cp_parts.len());

    let separator = if cfg!(windows) { ";" } else { ":" };
    let classpath  = cp_parts.join(separator);

    let mut cmd = Command::new("java");
    cmd.current_dir(&game_dir).arg("-Xmx2G");

    #[cfg(target_os = "macos")]
    cmd.arg("-XstartOnFirstThread");

    cmd.arg("-cp").arg(&classpath)
       .arg(&main_class)
       .arg("--username").arg(&name)
       .arg("--uuid").arg(&uuid)
       .arg("--accessToken").arg(&access_token)
       .arg("--version").arg(&version)
       .arg("--gameDir").arg(&game_dir)
       .arg("--assetsDir").arg(game_dir.join("assets"))
       .arg("--assetIndex").arg(&asset_index_id);

    eprintln!("[Launch] Lancement : {} avec mainClass {}", version, main_class);
    cmd.spawn().map_err(|e| format!("Erreur lancement Java : {}", e))?;
    Ok(())
}

#[tauri::command]
async fn start_microsoft_auth() -> Result<String, String> {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();

    thread::spawn(move || {
        let listener = match TcpListener::bind("127.0.0.1:8080") {
            Ok(l) => l,
            Err(e) => { let _ = tx.send(Err(format!("Impossible de démarrer le serveur : {}", e))); return; }
        };

        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) => { let _ = tx.send(Err(format!("Erreur d'acceptation : {}", e))); return; }
        };

        let mut buffer = [0; 8192];
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
            Err(e) => { let _ = tx.send(Err(format!("Erreur de lecture : {}", e))); return; }
        };

        let request = String::from_utf8_lossy(&buffer[..n]);
        eprintln!("[Rust] Requête reçue : {}", request.lines().next().unwrap_or(""));

        let code = if let Some(pos) = request.find("code=") {
            let rest = &request[pos + 5..];
            let raw  = rest.split(['&', ' ', '\r', '\n']).next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            raw.and_then(|c| urlencoding::decode(&c).map(|d| d.into_owned()).ok())
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
            Some(c) => { let _ = tx.send(Ok(c)); }
            None    => { let _ = tx.send(Err("Code non trouvé dans la requête".to_string())); }
        }
    });

    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(result) => result,
        Err(_)     => Err("Timeout : aucune réponse du navigateur".to_string()),
    }
}

#[tauri::command]
async fn exchange_microsoft_token(
    code: String,
    code_verifier: String,
    client_id: String,
    redirect_uri: String,
) -> Result<String, String> {
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("client_id",     &client_id)
        .append_pair("redirect_uri",  &redirect_uri)
        .append_pair("grant_type",    "authorization_code")
        .append_pair("code",          &code)
        .append_pair("code_verifier", &code_verifier)
        .append_pair("scope",         "XboxLive.signin openid offline_access")
        .finish();

    let client = reqwest::Client::new();
    let res = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send().await.map_err(|e| format!("Requête échouée : {}", e))?;

    let status = res.status();
    let text   = res.text().await.map_err(|e| e.to_string())?;

    if !status.is_success() {
        return Err(format!("Token error {}: {}", status, text));
    }

    Ok(text)
}

#[tauri::command]
async fn refresh_microsoft_token(
    refresh_token: String,
    client_id: String,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id",     client_id.as_str()),
        ("grant_type",    "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("scope",         "XboxLive.signin openid offline_access"),
    ];
    let res  = client
        .post("https://login.microsoftonline.com/consumers/oauth2/v2.0/token")
        .form(&params)
        .send().await.map_err(|e| e.to_string())?;
    let body = res.text().await.map_err(|e| e.to_string())?;
    Ok(body)
}

#[tauri::command]
async fn install_local_mod(source_path: String) -> Result<String, String> {
    let game_dir = get_game_dir()?;
    let mods_dir = game_dir.join("mods");
    std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

    let source   = std::path::Path::new(&source_path);
    let filename = source.file_name().ok_or("Nom de fichier invalide")?
        .to_string_lossy().into_owned();
    let dest = mods_dir.join(&filename);

    if dest.exists() { return Ok(dest.to_string_lossy().into_owned()); }

    std::fs::copy(&source, &dest).map_err(|e| format!("Erreur copie : {}", e))?;
    eprintln!("[Mod] ✅ Mod local installé : {}", filename);
    Ok(dest.to_string_lossy().into_owned())
}

#[tauri::command]
async fn list_mods() -> Result<Vec<String>, String> {
    let game_dir = get_game_dir()?;
    let mods_dir = game_dir.join("mods");
    if !mods_dir.exists() { return Ok(vec![]); }

    let mods = std::fs::read_dir(&mods_dir).map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("jar"))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    Ok(mods)
}

#[tauri::command]
async fn delete_mod(filename: String) -> Result<(), String> {
    let mod_path = get_game_dir()?.join("mods").join(&filename);
    if mod_path.exists() {
        std::fs::remove_file(&mod_path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn read_config(filename: String) -> Result<String, String> {
    let config_dir = get_game_dir()?.join("config");
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    let path = config_dir.join(&filename);
    if !path.exists() { return Ok("{}".to_string()); }
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

#[tauri::command]
async fn write_config(filename: String, content: String) -> Result<(), String> {
    let config_dir = get_game_dir()?.join("config");
    std::fs::create_dir_all(&config_dir).map_err(|e| e.to_string())?;
    std::fs::write(config_dir.join(&filename), content).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            start_microsoft_auth,
            exchange_microsoft_token,
            refresh_microsoft_token,
            launch_minecraft,
            install_version,
            install_fabric,
            install_local_mod,
            list_mods,
            delete_mod,
            read_config,
            write_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}