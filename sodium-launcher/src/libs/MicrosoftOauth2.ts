import { fetch } from '@tauri-apps/plugin-http';
import { invoke } from "@tauri-apps/api/core";

interface AuthorizationTokenResponse {
    token_type: string;
    expires_in: number;
    scope: string;
    access_token: string;
    refresh_token: string;
    user_id?: string;
    foci?: string;
}

interface XboxServiceTokenResponse {
    IssueInstant: string;
    NotAfter: string;
    Token: string;
    DisplayClaims: { xui: { uhs: string }[] };
}

interface MCTokenResponse {
    username: string;
    roles: any[];
    access_token: string;
    token_type: string;
    expires_in: number;
}

interface MCUserInfo {
    id: string;
    name: string;
    skins: any[];
    capes: any[];
}

interface MCVersion{
    id: string;
    type: "release" | "snapshot" | "old_beta" | "old_alpha";
    releaseTime: string;
}

export interface AuthInfo {
    auth_token: AuthorizationTokenResponse;
    xbox_token: XboxServiceTokenResponse;
    xsts_token: XboxServiceTokenResponse;
    mc_token: MCTokenResponse;
    mc_info: MCUserInfo;
}

export default class MicrosoftOauth2 {
    private clientId: string;
    private redirectUri: string;
    private codeVerifier: string = "";

    // URL définie une seule fois
    private static readonly TOKEN_URL = "https://login.microsoftonline.com/common/oauth2/v2.0/token";

    constructor(clientId: string, _clientSecret: string, redirectUri: string) {
        if (!clientId) throw new Error("clientId is required");
        if (!redirectUri) throw new Error("redirectUri is required");
        this.clientId = clientId;
        this.redirectUri = redirectUri;
    }

    // ── PKCE helpers ──────────────────────────────────────────────────────────

    private generateCodeVerifier(): string {
        const array = new Uint8Array(32);
        crypto.getRandomValues(array);
        return btoa(String.fromCharCode(...array))
            .replace(/\+/g, '-')
            .replace(/\//g, '_')
            .replace(/=/g, '');
    }

    private async generateCodeChallenge(verifier: string): Promise<string> {
        const data = new TextEncoder().encode(verifier);
        const digest = await crypto.subtle.digest('SHA-256', data);
        return btoa(String.fromCharCode(...new Uint8Array(digest)))
            .replace(/\+/g, '-')
            .replace(/\//g, '_')
            .replace(/=/g, '');
    }

    // ── URL d'autorisation ────────────────────────────────────────────────────

    public async getForwardUrl(): Promise<{ url: string, verifier: string }> {
        const verifier = this.generateCodeVerifier();
        const challenge = await this.generateCodeChallenge(verifier);

        const params = new URLSearchParams({
            client_id: this.clientId,
            response_type: "code",
            redirect_uri: this.redirectUri,
            scope: "XboxLive.signin openid offline_access",
            code_challenge: challenge,
            code_challenge_method: "S256",
        });

        return {
            url: `https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize?${params.toString()}`,
            verifier: verifier
        };
    }

    // ── Flow complet ──────────────────────────────────────────────────────────

    public async getAuthCodes(code: string, refresh: boolean = false, manualVerifier?: string): Promise<AuthInfo> {
        if (!code) throw new Error("No Code provided.");

        // Si on a passé un verifier manuel, on l'utilise
        if (manualVerifier) {
            this.codeVerifier = manualVerifier;
        }
        const authToken = await this.authCodeToAuthToken(code, refresh);
        const xbl       = await this.authTokenToXBL(authToken);
        const xsts      = await this.xblToXsts(xbl);
        const mcToken   = await this.xstsToMc(xsts);
        const mcInfo    = await this.getMCInfo(mcToken);

        return {
            auth_token: authToken,
            xbox_token: xbl,
            xsts_token: xsts,
            mc_token:   mcToken,
            mc_info:    mcInfo,
        };
    }

    // ── Étape 1 : code → token Microsoft ─────────────────────────────────────

    private async authCodeToAuthToken(code: string, refresh: boolean): Promise<AuthorizationTokenResponse> {
        if (refresh) {
            // ✅ Via Rust comme l'échange initial
            const jsonStr = await invoke<string>('refresh_microsoft_token', {
                refreshToken: code,
                clientId: this.clientId,
            });

            const result = JSON.parse(jsonStr);
            if (result.error) {
                throw new Error(`Refresh token error: ${result.error_description}`);
            }
            return result;
        }

        // Échange initial → via Rust (évite le header Origin bloqué par Microsoft)
        const jsonStr = await invoke<string>('exchange_microsoft_token', {
            code,
            codeVerifier: this.codeVerifier,
            clientId:     this.clientId,
            redirectUri:  this.redirectUri,
        });

        return JSON.parse(jsonStr);
    }

    // ── Étape 2 : token Microsoft → token XBL ────────────────────────────────

    private async authTokenToXBL(authToken: AuthorizationTokenResponse): Promise<XboxServiceTokenResponse> {
        const res = await fetch("https://user.auth.xboxlive.com/user/authenticate", {
            method:  "POST",
            headers: {
                "Content-Type": "application/json",
                "Accept":       "application/json",
            },
            body: JSON.stringify({
                Properties: {
                    AuthMethod: "RPS",
                    SiteName:   "user.auth.xboxlive.com",
                    RpsTicket:  `d=${authToken.access_token}`,
                },
                RelyingParty: "http://auth.xboxlive.com",
                TokenType:    "JWT",
            }),
        });

        if (!res.ok) {
            const err = await res.text();
            throw new Error(`XBL error: ${res.status} - ${err}`);
        }
        return res.json();
    }

    // ── Étape 3 : token XBL → token XSTS ─────────────────────────────────────

    private async xblToXsts(token: XboxServiceTokenResponse): Promise<XboxServiceTokenResponse> {
        const res = await fetch("https://xsts.auth.xboxlive.com/xsts/authorize", {
            method:  "POST",
            headers: {
                "Content-Type": "application/json",
                "Accept":       "application/json",
            },
            body: JSON.stringify({
                Properties: {
                    SandboxId:  "RETAIL",
                    UserTokens: [token.Token],
                },
                RelyingParty: "rp://api.minecraftservices.com/",
                TokenType:    "JWT",
            }),
        });

        if (!res.ok) {
            const err = await res.text();
            throw new Error(`XSTS error: ${res.status} - ${err}`);
        }
        return res.json();
    }

    // ── Étape 4 : token XSTS → token Minecraft ───────────────────────────────

    private async xstsToMc(token: XboxServiceTokenResponse): Promise<MCTokenResponse> {
        const uhs = token.DisplayClaims.xui[0].uhs;
        const res = await fetch("https://api.minecraftservices.com/authentication/login_with_xbox", {
            method:  "POST",
            headers: {
                "Content-Type": "application/json",
                "Accept":       "application/json",
            },
            body: JSON.stringify({
                identityToken: `XBL3.0 x=${uhs};${token.Token}`,
            }),
        });

        if (!res.ok) {
            const err = await res.text();
            throw new Error(`MC token error: ${res.status} - ${err}`);
        }
        return res.json();
    }

    // ── Étape 5 : profil Minecraft ────────────────────────────────────────────

    private async getMCInfo(mc_token: MCTokenResponse): Promise<MCUserInfo> {
        const res = await fetch("https://api.minecraftservices.com/minecraft/profile", {
            method:  "GET",
            headers: { "Authorization": `Bearer ${mc_token.access_token}` },
        });

        if (!res.ok) {
            const err = await res.text();
            throw new Error(`MC info error: ${res.status} - ${err}`);
        }
        return res.json();
    }

    // ── Refresh public ────────────────────────────────────────────────────────

    public async refreshAuth(refreshToken: string): Promise<AuthInfo> {
        return this.getAuthCodes(refreshToken, true);
    }

    public async getVersion(includeSnapshots: boolean = false): Promise<MCVersion[]>{
        const res = await fetch('https://launchermeta.mojang.com/mc/game/version_manifest.json');

        if (!res.ok) throw new Error(`Impossible de récupérer les versions : ${res.status}`);

        const data = await res.json() as {
            latest: {release: string; snapshot: string};
            versions: MCVersion[];
        };

        if(includeSnapshots) {
            return data.versions;
        }

        return data.versions.filter(v => v.type === "release");
    }
}