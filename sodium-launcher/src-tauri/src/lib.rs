use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::process::Command;
use url::form_urlencoded;
use dirs;
use serde::Deserialize;
use tauri::Emitter;
use tauri::Manager;

#[derive(Deserialize)]
struct LaunchOptions {
    version: String,
    access_token: String,
    uuid: String,
    name: String,
}

fn get_game_dir() -> Result<std::path::PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let base = dirs::home_dir().ok_or("Home introuvable")?;
        Ok(base.join("Library").join("Application Support").join("sodium"))
    }
    #[cfg(target_os = "windows")]
    {
        let base = dirs::data_dir().ok_or("AppData\\Roaming introuvable")?;
        Ok(base.join(".sodium"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let base = dirs::home_dir().ok_or("Home introuvable")?;
        Ok(base.join(".sodium"))
    }
}

/// Retourne le nom de l'OS tel qu'utilisé dans les JSON Mojang/Fabric
/// Mojang utilise "osx" pour macOS et "windows" pour Windows
fn mojang_os_name() -> &'static str {
    #[cfg(target_os = "macos")]    { "osx" }
    #[cfg(target_os = "windows")]  { "windows" }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))] { "linux" }
}

/// Extrait un JAR natif dans `dest_dir` sans dépendre du binaire `jar`
/// Utilise la crate zip intégrée via std (nécessite la dépendance `zip` dans Cargo.toml)
fn extract_native_jar(jar_path: &std::path::Path, dest_dir: &std::path::Path) -> Result<(), String> {
    let file  = std::fs::File::open(jar_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();

        // On ne garde que les fichiers natifs pertinents (.dll, .dylib, .so)
        // et on ignore les métadonnées META-INF
        if name.starts_with("META-INF") { continue; }
        let ext = std::path::Path::new(&name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if !matches!(ext, "dll" | "dylib" | "so") { continue; }

        // Aplatir : on ne conserve que le nom de fichier (pas le chemin interne)
        let filename = std::path::Path::new(&name)
            .file_name()
            .ok_or_else(|| format!("Nom invalide dans le JAR : {}", name))?;

        let out_path = dest_dir.join(filename);
        if out_path.exists() { continue; }

        let mut out_file = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
        eprintln!("[Natif] ✅ Extrait : {}", filename.to_string_lossy());
    }
    Ok(())
}

/// Évalue si une librairie doit être incluse selon ses règles `allow`/`disallow`
fn library_is_allowed(lib: &serde_json::Value) -> bool {
    let rules = match lib["rules"].as_array() {
        Some(r) => r,
        None    => return true, // Pas de règles = toujours incluse
    };

    let os = mojang_os_name();
    // On part du principe "refusé par défaut" s'il y a des règles
    let mut allowed = false;

    for rule in rules {
        let action   = rule["action"].as_str().unwrap_or("disallow");
        let os_name  = rule["os"]["name"].as_str();

        match (action, os_name) {
            // Règle globale sans restriction d'OS
            ("allow", None)    => { allowed = true; }
            ("disallow", None) => { allowed = false; }
            // Règle ciblant un OS précis
            ("allow", Some(target_os))    => { if target_os == os { allowed = true; } }
            ("disallow", Some(target_os)) => { if target_os == os { allowed = false; } }
            _ => {}
        }
    }
    allowed
}

#[tauri::command]
async fn install_version(window: tauri::Window, version: String) -> Result<(), String> {
    let game_dir = get_game_dir()?;

    let version_dir = game_dir.join("versions").join(&version);
    std::fs::create_dir_all(&version_dir).map_err(|e| e.to_string())?;

    let jar_path    = version_dir.join(format!("{}.jar", version));
    let json_path   = version_dir.join(format!("{}.json", version));
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

    let game_dir          = get_game_dir()?;
    let versions_dir      = game_dir.join("versions").join(&version);
    let libs_dir          = game_dir.join("libraries");
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

    // Clé native selon l'OS courant (convention Mojang)
    let natives_key = if cfg!(target_os = "windows")     { "natives-windows" }
                      else if cfg!(target_os = "macos")  { "natives-osx" }
                      else                               { "natives-linux" };

    let natives_dir = game_dir.join("natives");
    std::fs::create_dir_all(&natives_dir).map_err(|e| e.to_string())?;

    for lib in &all_libraries {
        // Filtrer selon les règles OS du manifest
        if !library_is_allowed(lib) {
            continue;
        }

        // Natifs : extraire les fichiers .dll / .dylib / .so, ne PAS ajouter au classpath
        if lib["natives"].is_object() {
            if let Some(classifier) = lib["downloads"]["classifiers"][natives_key].as_object() {
                let native_path_str = classifier["path"].as_str().unwrap_or("");
                if !native_path_str.is_empty() {
                    let native_full = libs_dir.join(native_path_str);
                    if native_full.exists() {
                        // Extraction sans dépendre du binaire `jar` externe
                        if let Err(e) = extract_native_jar(&native_full, &natives_dir) {
                            eprintln!("[Launch] ⚠️ Extraction native échouée ({}): {}", native_path_str, e);
                        } else {
                            eprintln!("[Launch] Natif extrait: {}", native_path_str);
                        }
                    }
                }
            }
            continue; // Ne pas ajouter les JARs natifs au classpath
        }

        // Librairie normale via downloads.artifact
        if let Some(path) = lib["downloads"]["artifact"]["path"].as_str() {
            let full = libs_dir.join(path);
            if full.exists() {
                cp_parts.push(full.to_string_lossy().into_owned());
            } else {
                eprintln!("[Launch] ⚠️ Manquante (vanilla): {}", path);
            }
            continue;
        }

        // Librairie Fabric (nom Maven, pas de downloads.artifact)
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

    // Séparateur de classpath : `;` sur Windows, `:` ailleurs
    let separator = if cfg!(target_os = "windows") { ";" } else { ":" };
    let classpath  = cp_parts.join(separator);

    let java_bin = find_java_binary();
    eprintln!("[Launch] Java: {}", java_bin);

    let mut cmd = Command::new(&java_bin);
    cmd.current_dir(&game_dir);
    cmd.arg("-Xmx2G");
    cmd.arg(format!("-Djava.library.path={}", natives_dir.to_string_lossy()));

    // -XstartOnFirstThread est requis uniquement sur macOS (contrainte LWJGL/OpenGL)
    #[cfg(target_os = "macos")]
    cmd.arg("-XstartOnFirstThread");

    // Sur Windows, on masque la fenêtre de console de javaw
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.arg("-cp").arg(&classpath)
       .arg(&main_class)
       .arg("--username").arg(&name)
       .arg("--uuid").arg(&uuid)
       .arg("--accessToken").arg(&access_token)
       .arg("--version").arg(&version)
       .arg("--gameDir").arg(&game_dir)
       .arg("--assetsDir").arg(game_dir.join("assets"))
       .arg("--assetIndex").arg(&asset_index_id)
       .arg("--userType").arg("msa");

    eprintln!("[Launch] Lancement : {} avec mainClass {}", version, main_class);

    let child = cmd.spawn()
        .map_err(|e| format!(
            "Erreur lancement Java ({}): {}. Java est-il installé et dans le PATH ?",
            java_bin, e
        ))?;

    eprintln!("[Launch] ✅ Processus démarré, PID: {}", child.id());
    Ok(())
}

/// Trouve le binaire Java selon l'OS et les variables d'environnement
fn find_java_binary() -> String {
    // 1. JAVA_HOME explicitement défini
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        #[cfg(target_os = "windows")]
        let bin = format!("{}\\bin\\javaw.exe", java_home);
        #[cfg(not(target_os = "windows"))]
        let bin = format!("{}/bin/java", java_home);

        if std::path::Path::new(&bin).exists() {
            return bin;
        }
    }

    // 2. Java livré avec le launcher (chemin relatif depuis le répertoire courant)
    #[cfg(target_os = "windows")]
    let bundled = "runtime\\java\\bin\\javaw.exe";
    #[cfg(not(target_os = "windows"))]
    let bundled = "runtime/java/bin/java";

    if std::path::Path::new(bundled).exists() {
        return bundled.to_string();
    }

    // 3. Fallback : Java du PATH système
    #[cfg(target_os = "windows")]
    { "javaw.exe".to_string() }
    #[cfg(not(target_os = "windows"))]
    { "java".to_string() }
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

#[tauri::command]
async fn install_bundled_mods(app: tauri::AppHandle) -> Result<(), String> {
    let game_dir = get_game_dir()?;
    let mods_dir = game_dir.join("mods");
    std::fs::create_dir_all(&mods_dir).map_err(|e| e.to_string())?;

    let mod_files = ["fabric-api.jar", "sodiummod.jar"];

    for filename in &mod_files {
        let dest = mods_dir.join(filename);
        if dest.exists() { continue; }

        let resource_path = app.path()
            .resource_dir()
            .map_err(|e| e.to_string())?
            .join("resources")
            .join("mods")
            .join(filename);

        std::fs::copy(&resource_path, &dest)
            .map_err(|e| format!("Erreur copie {} : {}", filename, e))?;

        eprintln!("[Mods] ✅ Installé : {}", filename);
    }

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            window.set_resizable(false).unwrap();
            window.set_maximizable(false).unwrap();
            Ok(())
        })
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
            install_bundled_mods,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}