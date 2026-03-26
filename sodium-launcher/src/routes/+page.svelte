<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

  let menuOpen = $state(false);
  let isConnected = $state(false);
  let username = $state("");
  let isLoading = $state(false);

  getCurrentWindow().setResizable(false);

  function toggleMenu() {
    menuOpen = !menuOpen;
  }

  async function connectMicrosoft() {
    isLoading = true;
    menuOpen = false;

    const CLIENT_ID = "dd919c86-6d2d-4471-a06b-1eac8f6d35a8"; // ← remplace ici
    const REDIRECT_URI = "http://localhost";
    const scope = "XboxLive.signin offline_access";

    const authUrl =
            `https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize` +
            `?client_id=${CLIENT_ID}` +
            `&response_type=code` +
            `&redirect_uri=${encodeURIComponent(REDIRECT_URI)}` +
            `&scope=${encodeURIComponent(scope)}` +
            `&response_mode=query`;

    // Ouvre une fenêtre Tauri avec la page Microsoft
    const authWindow = new WebviewWindow("microsoft-auth", {
      url: authUrl,
      title: "Connexion Microsoft",
      width: 500,
      height: 680,
      center: true,
      resizable: false,
    });

    // Écoute les changements d'URL pour capturer le code OAuth
    const unlisten = await authWindow.listen("tauri://navigation", async (event: any) => {
      const url: string = event.payload.url ?? event.payload ?? "";

      if (url.startsWith(REDIRECT_URI) && url.includes("code=")) {
        const code = new URL(url).searchParams.get("code");
        if (code) {
          await unlisten();
          authWindow.close();
          await handleAuthCode(code, CLIENT_ID, REDIRECT_URI);
        }
      }
    });

    authWindow.onCloseRequested(() => {
      isLoading = false;
    });
  }

  async function handleAuthCode(code: string, clientId: string, redirectUri: string) {
    try {
      // Étape 1 — Échange code → token Microsoft
      const msRes = await fetch(
              "https://login.microsoftonline.com/consumers/oauth2/v2.0/token",
              {
                method: "POST",
                headers: { "Content-Type": "application/x-www-form-urlencoded" },
                body: new URLSearchParams({
                  client_id: clientId,
                  code,
                  redirect_uri: redirectUri,
                  grant_type: "authorization_code",
                  scope: "XboxLive.signin offline_access",
                }),
              }
      );
      const msToken = await msRes.json();

      // Étape 2 — Token Microsoft → Xbox Live
      const xblRes = await fetch(
              "https://user.auth.xboxlive.com/user/authenticate",
              {
                method: "POST",
                headers: { "Content-Type": "application/json", Accept: "application/json" },
                body: JSON.stringify({
                  Properties: {
                    AuthMethod: "RPS",
                    SiteName: "user.auth.xboxlive.com",
                    RpsTicket: `d=${msToken.access_token}`,
                  },
                  RelyingParty: "http://auth.xboxlive.com",
                  TokenType: "JWT",
                }),
              }
      );
      const xblData = await xblRes.json();
      const xblToken = xblData.Token;
      const userHash = xblData.DisplayClaims.xui[0].uhs;

      // Étape 3 — Xbox Live → XSTS
      const xstsRes = await fetch("https://xsts.auth.xboxlive.com/xsts/authorize", {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify({
          Properties: {
            SandboxId: "RETAIL",
            UserTokens: [xblToken],
          },
          RelyingParty: "rp://api.minecraftservices.com/",
          TokenType: "JWT",
        }),
      });
      const xstsData = await xstsRes.json();
      const xstsToken = xstsData.Token;

      // Étape 4 — XSTS → token Minecraft
      const mcRes = await fetch(
              "https://api.minecraftservices.com/authentication/login_with_xbox",
              {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                  identityToken: `XBL3.0 x=${userHash};${xstsToken}`,
                }),
              }
      );
      const mcData = await mcRes.json();
      const mcToken = mcData.access_token;

      // Étape 5 — Récupère le profil Minecraft
      const profileRes = await fetch(
              "https://api.minecraftservices.com/minecraft/profile",
              {
                headers: { Authorization: `Bearer ${mcToken}` },
              }
      );
      const profile = await profileRes.json();

      username = profile.name;
      isConnected = true;
    } catch (err) {
      console.error("Erreur auth:", err);
    } finally {
      isLoading = false;
    }
  }
</script>

<div class="app-wrapper">
  {#if !menuOpen}
    <button class="hamburger-open" onclick={toggleMenu} aria-label="Ouvrir le menu">
      <span></span>
      <span></span>
      <span></span>
    </button>
  {/if}

  <nav class="sidebar" class:sidebar-open={menuOpen}>
    <div class="sidebar-top">
      <button class="hamburger" onclick={toggleMenu} aria-label="Fermer le menu">
        <span class:open={menuOpen}></span>
        <span class:open={menuOpen}></span>
        <span class:open={menuOpen}></span>
      </button>
      <h2>Menu</h2>
      <ul>
        <li><a href="/" onclick={toggleMenu}>Accueil</a></li>
        <li><a href="/parametres" onclick={toggleMenu}>Paramètres</a></li>
        <li><a href="/aide" onclick={toggleMenu}>Aide</a></li>
      </ul>
    </div>

    <div class="sidebar-bottom">
      {#if isConnected}
        <div class="user-info">
          <span class="avatar">👤</span>
          <span class="mc-username">{username}</span>
        </div>
      {:else}
        <button
                class="connexion-btn"
                onclick={connectMicrosoft}
                disabled={isLoading}
        >
          {#if isLoading}
            <span class="spinner"></span> Connexion...
          {:else}
            👤 Connexion
          {/if}
        </button>
      {/if}
    </div>
  </nav>

  <main class="container" class:content-shifted={menuOpen}>
    <!-- Ton contenu ici -->
    {#if isConnected}
      <p>Connecté en tant que <strong>{username}</strong></p>
    {/if}
  </main>
</div>

<style>
  /* ... tout ton CSS existant ... */

  .user-info {
    display: flex;
    align-items: center;
    gap: 0.8vw;
    padding: 1.2vh 1vw;
    background: #e8f0fe;
    border-radius: 8px;
    font-weight: 600;
    font-size: clamp(0.8rem, 1vw, 1rem);
    color: #396cd8;
  }

  .mc-username {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .connexion-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.8vw;
    width: 100%;
    padding: 1.2vh 1vw;
    background: #396cd8;
    color: #ffffff;
    border: none;
    border-radius: 8px;
    font-weight: 600;
    font-size: clamp(0.8rem, 1vw, 1rem);
    text-decoration: none;
    cursor: pointer;
    transition: background 0.2s;
    box-sizing: border-box;
  }

  .connexion-btn:hover:not(:disabled) {
    background: #2a55b8;
  }

  .connexion-btn:disabled {
    opacity: 0.7;
    cursor: not-allowed;
  }

  .spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid #ffffff44;
    border-top-color: #ffffff;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>