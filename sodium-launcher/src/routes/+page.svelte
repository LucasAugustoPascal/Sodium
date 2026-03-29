<script lang="ts">
    import MicrosoftOauth2 from "../libs/MicrosoftOauth2";
    import { invoke } from '@tauri-apps/api/core';
    import { open } from '@tauri-apps/plugin-shell';
    import { load } from '@tauri-apps/plugin-store';
    import Button, { Label, Icon } from '@smui/button';
    import Card, { Content, Actions } from '@smui/card';

    const auth = new MicrosoftOauth2(
        "dd919c86-6d2d-4471-a06b-1eac8f6d35a8",
        import.meta.env.VITE_CLIENT_SECRET,
        "http://127.0.0.1:8080"
    );

    const DEFAULT_AVATAR = "https://mc-heads.net/avatar/steve/100";

    interface MinecraftInstance {
        id: string;
        name: string;
        version: string;
        lastPlayed?: string;
    }

    interface MCVersion {
        id: string;
        type: "release" | "snapshot" | "old_beta" | "old_alpha";
        releaseTime: string;
    }

    let username = $state("");
    let avatar = $state(DEFAULT_AVATAR);
    let loading = $state(false);
    let error = $state("");
    let authInProgress = $state(false);
    let activePage = $state("play");

    // Instances
    let instances = $state<MinecraftInstance[]>([]);
    let selectedInstanceId = $state<string | null>(null);
    let showAddModal = $state(false);
    let newInstanceName = $state("");
    let newInstanceVersion = $state("1.21.4");

    // Versions Mojang
    let availableVersions = $state<MCVersion[]>([]);
    let showSnapshots = $state(false);
    let versionsLoading = $state(false);

    const filteredVersions = $derived(
        showSnapshots
            ? availableVersions
            : availableVersions.filter(v => v.type === "release")
    );

    const selectedInstance = $derived(
        instances.find(i => i.id === selectedInstanceId) ?? null
    );

    async function fetchVersions() {
        versionsLoading = true;
        try {
            const versions = await auth.getVersion(true)
            availableVersions = versions;
            if (!newInstanceVersion) {
                newInstanceVersion = versions.find(v => v.type === "release")?.id ?? "1.21.4";
            }
        } catch (e) {
            console.error("❌ Impossible de récupérer les versions :", e);
            // Fallback
            availableVersions = [
                { id: "1.21.4", type: "release", releaseTime: "" },
                { id: "1.21.1", type: "release", releaseTime: "" },
                { id: "1.20.4", type: "release", releaseTime: "" },
                { id: "1.20.1", type: "release", releaseTime: "" },
                { id: "1.19.4", type: "release", releaseTime: "" },
                { id: "1.18.2", type: "release", releaseTime: "" },
                { id: "1.17.1", type: "release", releaseTime: "" },
                { id: "1.16.5", type: "release", releaseTime: "" },
            ];
        } finally {
            versionsLoading = false;
        }
    }

    $effect(() => {
        (async () => {
            try {
                // Session
                const sessionStore = await load('session.json', { autoSave: true, defaults: {} });
                const session = await sessionStore.get<{
                    username: string;
                    avatar: string;
                    refresh_token: string;
                }>('mc_session');

                if (session?.username) {
                    username = session.username;
                    avatar = session.avatar || DEFAULT_AVATAR;

                    if (session.refresh_token) {
                        try {
                            const newAuth = await auth.refreshAuth(session.refresh_token);
                            await sessionStore.set('mc_session', {
                                username, avatar,
                                refresh_token: newAuth.auth_token.refresh_token
                            });
                            console.log("✅ Token renouvelé");
                        } catch (e) {
                            console.error("❌ Refresh échoué :", e);
                            username = ""; avatar = DEFAULT_AVATAR;
                            await sessionStore.delete('mc_session');
                        }
                    }
                }

                // Instances
                const instanceStore = await load('instances.json', { autoSave: true, defaults: {} });
                const saved = await instanceStore.get<{ list: MinecraftInstance[]; selectedId: string }>('instances');
                if (saved?.list?.length) {
                    instances = saved.list;
                    selectedInstanceId = saved.selectedId ?? saved.list[0].id;
                }

                // Versions Mojang
                await fetchVersions();

            } catch (e) {
                console.error("❌ Erreur chargement :", e);
            }
        })();
    });

    async function saveInstances() {
        const instanceStore = await load('instances.json', { autoSave: true, defaults: {} });
        await instanceStore.set('instances', { list: instances, selectedId: selectedInstanceId });
    }

    async function addInstance() {
        if (!newInstanceName.trim()) return;
        const newInst: MinecraftInstance = {
            id: crypto.randomUUID(),
            name: newInstanceName.trim(),
            version: newInstanceVersion,
            lastPlayed: undefined
        };
        instances = [...instances, newInst];
        selectedInstanceId = newInst.id;
        newInstanceName = "";
        newInstanceVersion = availableVersions.find(v => v.type === "release")?.id ?? "1.21.4";
        showAddModal = false;
        await saveInstances();
    }

    async function deleteInstance(id: string) {
        instances = instances.filter(i => i.id !== id);
        if (selectedInstanceId === id) {
            selectedInstanceId = instances[0]?.id ?? null;
        }
        await saveInstances();
    }

    async function login() {
        if (authInProgress) return;
        authInProgress = true; loading = true; error = "";
        try {
            const codePromise = invoke<string>('start_microsoft_auth');
            const { url, verifier } = await auth.getForwardUrl();
            await open(url);
            const code = await codePromise;
            if (!code || code.length < 10) throw new Error("Code invalide");
            await handleCode(code, verifier);
        } catch (e: any) {
            error = "Erreur de connexion avec Microsoft.";
        } finally {
            loading = false; authInProgress = false;
        }
    }

    async function handleCode(code: string, verifier: string) {
        try {
            const authInfo = await auth.getAuthCodes(code, false, verifier);
            username = authInfo.mc_info?.name || "";
            let uuid = authInfo.mc_info?.id;
            if (uuid) {
                uuid = uuid.replace(/-/g, '');
                avatar = `https://mc-heads.net/avatar/${uuid}/100`;
            } else { avatar = DEFAULT_AVATAR; }
            const store = await load('session.json', { autoSave: true, defaults: {} });
            await store.set('mc_session', {
                username, avatar,
                refresh_token: authInfo.auth_token.refresh_token
            });
        } catch (e: any) {
            error = "Échec de l'authentification Minecraft.";
            avatar = DEFAULT_AVATAR;
        }
    }

    function handleImageError() {
        if (avatar !== DEFAULT_AVATAR) avatar = DEFAULT_AVATAR;
    }

    async function logout() {
        try {
            const store = await load('session.json', { autoSave: true, defaults: {} });
            await store.delete('mc_session');
        } catch (e) {}
        username = ""; avatar = DEFAULT_AVATAR; error = "";
    }

    async function launchGame() {
        if (!selectedInstance) return;
        instances = instances.map(i =>
            i.id === selectedInstance.id
                ? { ...i, lastPlayed: new Date().toLocaleDateString('fr-FR') }
                : i
        );
        await saveInstances();
        console.log(`🚀 Lancement → ${selectedInstance.name} | ${selectedInstance.version}`);
    }

    const navItems = [
        { id: "play",     icon: "sports_esports", label: "Jouer" },
        { id: "mods",     icon: "extension",      label: "Mods" },
        { id: "settings", icon: "settings",       label: "Paramètres" },
    ];
</script>

{#if username}
    <div class="app-layout">

        <!-- ═══ SIDEBAR ═══ -->
        <aside class="sidebar">
            <div class="sidebar-logo">
                <div class="logo-icon">
                    <span class="material-icons">grass</span>
                </div>
                <span class="logo-text">Sodium<span class="accent">MC</span></span>
            </div>

            <nav class="sidebar-nav">
                {#each navItems as item}
                    <button
                            class="nav-item"
                            class:active={activePage === item.id}
                            onclick={() => activePage = item.id}
                    >
                        <span class="material-icons">{item.icon}</span>
                        <span class="nav-label">{item.label}</span>
                    </button>
                {/each}
            </nav>

            <div class="sidebar-profile">
                <div class="profile-avatar">
                    <img src={avatar} alt="Avatar de {username}" class="pixelated" onerror={handleImageError} />
                </div>
                <div class="profile-info">
                    <span class="profile-name">{username}</span>
                    <span class="profile-status">En ligne</span>
                </div>
                <button class="logout-icon" onclick={logout} title="Déconnexion">
                    <span class="material-icons">logout</span>
                </button>
            </div>
        </aside>

        <!-- ═══ CONTENU ═══ -->
        <main class="main-content">

            <!-- PAGE JOUER -->
            {#if activePage === "play"}
                <div class="page-play">
                    <div class="play-header">
                        <div>
                            <h1>Bonjour, <span class="accent">{username}</span> 👋</h1>
                            <p>Sélectionne une instance et lance le jeu.</p>
                        </div>
                        <button class="btn-add" onclick={() => showAddModal = true}>
                            <span class="material-icons">add</span>
                            Nouvelle instance
                        </button>
                    </div>

                    {#if instances.length === 0}
                        <div class="empty-state">
                            <span class="material-icons empty-icon">inbox</span>
                            <p>Aucune instance pour le moment.</p>
                            <button class="btn-add" onclick={() => showAddModal = true}>
                                <span class="material-icons">add</span>
                                Créer une instance
                            </button>
                        </div>
                    {:else}
                        <div class="instances-list">
                            {#each instances as inst}
                                <div
                                        class="instance-card"
                                        class:selected={selectedInstanceId === inst.id}
                                        onclick={() => selectedInstanceId = inst.id}
                                        role="button"
                                        tabindex="0"
                                >
                                    <div class="instance-icon">
                                        <span class="material-icons">grass</span>
                                    </div>
                                    <div class="instance-info">
                                        <span class="instance-name">{inst.name}</span>
                                        <span class="instance-meta">
                                            Java Edition · {inst.version}
                                            {#if inst.lastPlayed}· Joué le {inst.lastPlayed}{/if}
                                        </span>
                                    </div>
                                    <div class="instance-actions">
                                        {#if selectedInstanceId === inst.id}
                                            <button class="btn-play" onclick={(e) => { e.stopPropagation(); launchGame(); }}>
                                                <span class="material-icons">play_arrow</span>
                                                Lancer
                                            </button>
                                        {/if}
                                        <button class="btn-delete" onclick={(e) => { e.stopPropagation(); deleteInstance(inst.id); }} title="Supprimer">
                                            <span class="material-icons">delete_outline</span>
                                        </button>
                                    </div>
                                </div>
                            {/each}
                        </div>
                    {/if}
                </div>

                <!-- PAGE MODS / SETTINGS -->
            {:else if activePage === "mods" || activePage === "settings"}
                <div class="page-placeholder">
                    <span class="material-icons placeholder-icon">
                        {activePage === "mods" ? "extension" : "settings"}
                    </span>
                    <h2>{activePage === "mods" ? "Mods" : "Paramètres"}</h2>
                    <p>Bientôt disponible</p>
                </div>
            {/if}
        </main>
    </div>

    <!-- ═══ MODAL NOUVELLE INSTANCE ═══ -->
    {#if showAddModal}
        <div class="modal-overlay" onclick={() => showAddModal = false} role="button" tabindex="0">
            <div class="modal" onclick={(e) => e.stopPropagation()} role="dialog">
                <div class="modal-header">
                    <h2>Nouvelle instance</h2>
                    <button class="modal-close" onclick={() => showAddModal = false}>
                        <span class="material-icons">close</span>
                    </button>
                </div>

                <div class="modal-body">
                    <div class="config-group">
                        <label class="config-label">Nom de l'instance</label>
                        <input
                                type="text"
                                bind:value={newInstanceName}
                                placeholder="Ex: Survival Moddé"
                                class="config-input"
                        />
                    </div>

                    <div class="config-group">
                        <div class="label-row">
                            <label class="config-label">Version de Minecraft</label>
                            <label class="snapshot-toggle">
                                <input type="checkbox" bind:checked={showSnapshots} />
                                Snapshots
                            </label>
                        </div>
                        {#if versionsLoading}
                            <div class="versions-loading">
                                <span class="spinner-sm"></span>
                                Chargement des versions...
                            </div>
                        {:else}
                            <select bind:value={newInstanceVersion} class="config-select">
                                {#each filteredVersions as ver}
                                    <option value={ver.id}>
                                        {ver.id}{ver.type !== "release" ? " · snapshot" : ""}
                                    </option>
                                {/each}
                            </select>
                        {/if}
                    </div>
                </div>

                <div class="modal-footer">
                    <button class="btn-cancel" onclick={() => showAddModal = false}>Annuler</button>
                    <button class="btn-confirm" onclick={addInstance} disabled={!newInstanceName.trim()}>
                        <span class="material-icons">add</span>
                        Créer
                    </button>
                </div>
            </div>
        </div>
    {/if}

{:else}
    <!-- ═══ LOGIN ═══ -->
    <div class="login-container">
        <div class="card-wrapper">
            <Card class="glass-card">
                <Content>
                    <div class="login-header">
                        <div class="logo-icon-big">
                            <span class="material-icons">grass</span>
                        </div>
                        <h1 class="app-title">Sodium<span class="accent">MC</span></h1>
                        <p class="app-subtitle">
                            Connectez-vous avec votre compte Microsoft pour accéder au jeu.
                        </p>
                    </div>
                    {#if error}
                        <div class="error-banner">
                            <span class="material-icons error-icon">error_outline</span>
                            <span>{error}</span>
                        </div>
                    {/if}
                </Content>
                <Actions class="login-actions">
                    <Button variant="raised" class="microsoft-btn" onclick={login} disabled={loading}>
                        {#if loading}
                            <span class="spinner"></span>
                        {:else}
                            <svg class="ms-logo" viewBox="0 0 23 23" xmlns="http://www.w3.org/2000/svg">
                                <path d="M0 0v11h11V0H0zm12 0v11h11V0H12zM0 12v11h11V12H0zm12 0v11h11V12H12z"/>
                            </svg>
                        {/if}
                        <Label>{loading ? 'Connexion en cours...' : 'Se connecter avec Microsoft'}</Label>
                    </Button>
                </Actions>
            </Card>
        </div>
    </div>
{/if}

<style lang="postcss">
    :global(body) {
        margin: 0; padding: 0; overflow: hidden;
        font-family: 'Roboto', 'Segoe UI', sans-serif;
        background: #0a0a0f; color: #fff;
    }

    /* ═══════════════════════════
       LAYOUT
    ═══════════════════════════ */
    .app-layout { display: flex; height: 100vh; width: 100vw; overflow: hidden; }

    /* ═══════════════════════════
       SIDEBAR
    ═══════════════════════════ */
    .sidebar {
        width: 220px; min-width: 220px; height: 100vh;
        background: rgba(8, 8, 18, 0.98);
        border-right: 1px solid rgba(255,255,255,0.05);
        display: flex; flex-direction: column;
        padding: 1.25rem 0.75rem;
        box-sizing: border-box;
    }

    .sidebar-logo {
        display: flex; align-items: center; gap: 0.6rem;
        padding: 0.25rem 0.5rem 1rem;
        border-bottom: 1px solid rgba(255,255,255,0.06);
        margin-bottom: 0.75rem;
    }

    .logo-icon {
        width: 34px; height: 34px; border-radius: 10px;
        background: linear-gradient(135deg, rgba(99,0,255,0.4), rgba(0,200,150,0.3));
        display: flex; align-items: center; justify-content: center; flex-shrink: 0;
    }
    .logo-icon .material-icons { font-size: 18px; color: #00c896; }
    .logo-text { font-size: 1rem; font-weight: 800; color: #fff; letter-spacing: -0.02em; }

    .sidebar-nav { display: flex; flex-direction: column; gap: 0.2rem; flex: 1; }

    .nav-item {
        display: flex; align-items: center; gap: 0.75rem;
        padding: 0.65rem 0.75rem; border-radius: 10px; border: none;
        background: transparent; color: rgba(255,255,255,0.38);
        cursor: pointer; transition: all 0.15s;
        font-size: 0.875rem; font-weight: 500; width: 100%; text-align: left;
    }
    .nav-item:hover { background: rgba(255,255,255,0.05); color: rgba(255,255,255,0.75); }
    .nav-item.active { background: rgba(0,200,150,0.1); color: #00c896; }
    .nav-item .material-icons { font-size: 20px; }

    .sidebar-profile {
        display: flex; align-items: center; gap: 0.6rem;
        padding: 0.75rem 0.5rem;
        border-top: 1px solid rgba(255,255,255,0.06);
    }
    .profile-avatar {
        width: 34px; height: 34px; border-radius: 8px;
        overflow: hidden; flex-shrink: 0; background: rgba(0,0,0,0.3);
    }
    .profile-avatar img { width: 100%; height: 100%; object-fit: cover; }
    .profile-info { flex: 1; min-width: 0; display: flex; flex-direction: column; }
    .profile-name { font-size: 0.8rem; font-weight: 600; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .profile-status { font-size: 0.7rem; color: #00c896; }
    .logout-icon {
        background: none; border: none; color: rgba(255,255,255,0.25);
        cursor: pointer; padding: 4px; border-radius: 6px;
        display: flex; align-items: center; transition: all 0.15s;
    }
    .logout-icon:hover { color: #cf6679; background: rgba(207,102,121,0.1); }

    /* ═══════════════════════════
       MAIN CONTENT
    ═══════════════════════════ */
    .main-content {
        flex: 1; overflow-y: auto;
        background:
                radial-gradient(ellipse at 15% 25%, rgba(99,0,255,0.07) 0%, transparent 55%),
                radial-gradient(ellipse at 85% 75%, rgba(0,200,150,0.05) 0%, transparent 50%),
                #0d1117;
        padding: 2rem 2.5rem; box-sizing: border-box;
    }

    /* ═══════════════════════════
       PAGE JOUER
    ═══════════════════════════ */
    .page-play { display: flex; flex-direction: column; gap: 1.75rem; max-width: 800px; }

    .play-header {
        display: flex; align-items: center; justify-content: space-between; gap: 1rem;
    }
    .play-header h1 { font-size: 1.7rem; font-weight: 800; margin: 0 0 0.2rem; }
    .play-header p { color: rgba(255,255,255,0.38); font-size: 0.875rem; margin: 0; }

    .btn-add {
        display: flex; align-items: center; gap: 0.4rem;
        background: rgba(0,200,150,0.08); border: 1px solid rgba(0,200,150,0.22);
        color: #00c896; border-radius: 10px;
        padding: 0.55rem 1rem; font-size: 0.875rem; font-weight: 600;
        cursor: pointer; transition: all 0.15s; white-space: nowrap;
    }
    .btn-add:hover { background: rgba(0,200,150,0.15); border-color: rgba(0,200,150,0.45); }
    .btn-add .material-icons { font-size: 18px; }

    /* Instances */
    .instances-list { display: flex; flex-direction: column; gap: 0.55rem; }

    .instance-card {
        display: flex; align-items: center; gap: 1rem;
        background: rgba(17,24,39,0.55);
        border: 1px solid rgba(255,255,255,0.055);
        border-radius: 14px; padding: 1rem 1.25rem;
        cursor: pointer; transition: all 0.18s ease;
        backdrop-filter: blur(8px);
        position: relative; overflow: hidden;
    }
    .instance-card::before {
        content: ''; position: absolute; left: 0; top: 0; bottom: 0;
        width: 3px; background: transparent;
        border-radius: 14px 0 0 14px; transition: background 0.18s;
    }
    .instance-card:hover {
        background: rgba(17,24,39,0.8);
        border-color: rgba(255,255,255,0.1);
        transform: translateX(2px);
    }
    .instance-card.selected {
        background: rgba(0,200,150,0.06);
        border-color: rgba(0,200,150,0.28);
    }
    .instance-card.selected::before {
        background: linear-gradient(to bottom, #00c896, #00a8ff);
    }

    .instance-icon {
        width: 42px; height: 42px; border-radius: 10px;
        background: rgba(0,200,150,0.08);
        border: 1px solid rgba(0,200,150,0.15);
        display: flex; align-items: center; justify-content: center; flex-shrink: 0;
        transition: all 0.18s;
    }
    .instance-card.selected .instance-icon {
        background: rgba(0,200,150,0.14);
        border-color: rgba(0,200,150,0.35);
    }
    .instance-icon .material-icons { font-size: 22px; color: #00c896; }

    .instance-info { flex: 1; min-width: 0; }
    .instance-name { display: block; font-size: 0.95rem; font-weight: 600; color: #fff; }
    .instance-meta { display: block; font-size: 0.78rem; color: rgba(255,255,255,0.32); margin-top: 0.15rem; }

    .instance-actions { display: flex; align-items: center; gap: 0.5rem; }

    .btn-play {
        display: flex; align-items: center; gap: 0.35rem;
        background: linear-gradient(135deg, #00c896, #00a8ff);
        border: none; border-radius: 8px;
        color: #fff; font-size: 0.85rem; font-weight: 700;
        padding: 0.45rem 1rem; cursor: pointer; transition: all 0.15s;
        box-shadow: 0 3px 12px rgba(0,200,150,0.28);
    }
    .btn-play:hover { box-shadow: 0 5px 20px rgba(0,200,150,0.45); transform: translateY(-1px); }
    .btn-play .material-icons { font-size: 18px; }

    .btn-delete {
        background: none; border: none;
        color: rgba(255,255,255,0.18); cursor: pointer;
        padding: 6px; border-radius: 7px;
        display: flex; align-items: center; transition: all 0.15s;
    }
    .btn-delete:hover { color: #cf6679; background: rgba(207,102,121,0.1); }
    .btn-delete .material-icons { font-size: 18px; }

    /* Empty state */
    .empty-state {
        display: flex; flex-direction: column; align-items: center;
        justify-content: center; gap: 1rem; padding: 4rem 2rem; text-align: center;
        background: rgba(17,24,39,0.35);
        border: 1px dashed rgba(255,255,255,0.07);
        border-radius: 16px; color: rgba(255,255,255,0.28);
    }
    .empty-icon { font-size: 3.5rem; color: rgba(255,255,255,0.08); }
    .empty-state p { margin: 0; font-size: 0.9rem; }

    /* ═══════════════════════════
       MODAL
    ═══════════════════════════ */
    .modal-overlay {
        position: fixed; inset: 0;
        background: rgba(0,0,0,0.65);
        backdrop-filter: blur(5px);
        display: flex; align-items: center; justify-content: center;
        z-index: 100;
    }
    .modal {
        background: #111827;
        border: 1px solid rgba(255,255,255,0.09);
        border-radius: 18px; width: 420px; max-width: 92vw;
        box-shadow: 0 30px 70px rgba(0,0,0,0.65); overflow: hidden;
    }
    .modal-header {
        display: flex; align-items: center; justify-content: space-between;
        padding: 1.25rem 1.5rem;
        border-bottom: 1px solid rgba(255,255,255,0.07);
    }
    .modal-header h2 { margin: 0; font-size: 1rem; font-weight: 700; }
    .modal-close {
        background: none; border: none; color: rgba(255,255,255,0.35);
        cursor: pointer; padding: 4px; border-radius: 6px;
        display: flex; align-items: center; transition: all 0.15s;
    }
    .modal-close:hover { color: #fff; background: rgba(255,255,255,0.08); }

    .modal-body { padding: 1.5rem; display: flex; flex-direction: column; gap: 1.25rem; }

    .modal-footer {
        display: flex; justify-content: flex-end; gap: 0.75rem;
        padding: 1rem 1.5rem;
        border-top: 1px solid rgba(255,255,255,0.07);
    }

    .config-group { display: flex; flex-direction: column; gap: 0.4rem; }

    .label-row {
        display: flex; align-items: center; justify-content: space-between;
    }

    .config-label { font-size: 0.82rem; color: rgba(255,255,255,0.5); font-weight: 500; }

    .snapshot-toggle {
        display: flex; align-items: center; gap: 0.3rem;
        font-size: 0.75rem; color: rgba(255,255,255,0.35);
        cursor: pointer; font-weight: 400;
    }
    .snapshot-toggle input { accent-color: #00c896; cursor: pointer; }

    .config-input, .config-select {
        width: 100%; padding: 0.75rem 1rem;
        background: rgba(0,0,0,0.45);
        border: 1px solid rgba(255,255,255,0.09);
        border-radius: 10px; color: #fff;
        font-size: 0.95rem; box-sizing: border-box; transition: all 0.2s;
    }
    .config-input:focus, .config-select:focus {
        border-color: #00c896;
        box-shadow: 0 0 0 3px rgba(0,200,150,0.12); outline: none;
    }
    .config-select option { background: #1a2332; }

    .versions-loading {
        display: flex; align-items: center; gap: 0.5rem;
        padding: 0.75rem 1rem;
        background: rgba(0,0,0,0.3); border-radius: 10px;
        color: rgba(255,255,255,0.4); font-size: 0.875rem;
    }

    .btn-cancel {
        background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.09);
        color: rgba(255,255,255,0.55); border-radius: 9px;
        padding: 0.55rem 1.1rem; font-size: 0.875rem; font-weight: 500;
        cursor: pointer; transition: all 0.15s;
    }
    .btn-cancel:hover { background: rgba(255,255,255,0.08); }

    .btn-confirm {
        display: flex; align-items: center; gap: 0.35rem;
        background: linear-gradient(135deg, #00c896, #00a8ff);
        border: none; border-radius: 9px;
        color: #fff; font-size: 0.875rem; font-weight: 700;
        padding: 0.55rem 1.25rem; cursor: pointer; transition: all 0.15s;
    }
    .btn-confirm:hover:not(:disabled) { box-shadow: 0 4px 16px rgba(0,200,150,0.35); }
    .btn-confirm:disabled { opacity: 0.35; cursor: not-allowed; }
    .btn-confirm .material-icons { font-size: 16px; }

    /* ═══════════════════════════
       PLACEHOLDER
    ═══════════════════════════ */
    .page-placeholder {
        display: flex; flex-direction: column; align-items: center;
        justify-content: center; height: 60vh; gap: 1rem;
        color: rgba(255,255,255,0.18); text-align: center;
    }
    .placeholder-icon { font-size: 4rem; }
    .page-placeholder h2 { margin: 0; font-size: 1.2rem; color: rgba(255,255,255,0.22); }
    .page-placeholder p { margin: 0; font-size: 0.85rem; }

    /* ═══════════════════════════
       LOGIN
    ═══════════════════════════ */
    .login-container {
        min-height: 100vh; width: 100%; display: flex; align-items: center; justify-content: center;
        background:
                radial-gradient(ellipse at 20% 50%, rgba(99,0,255,0.15) 0%, transparent 60%),
                radial-gradient(ellipse at 80% 20%, rgba(0,200,150,0.1) 0%, transparent 50%),
                linear-gradient(135deg, #0a0a0f 0%, #111827 50%, #0a0a0f 100%);
    }
    .card-wrapper { width: 100%; max-width: 360px; padding: 1rem; }

    :global(.glass-card) {
        background: rgba(17,24,39,0.7) !important;
        backdrop-filter: blur(20px) !important;
        border: 1px solid rgba(255,255,255,0.08) !important;
        border-radius: 20px !important;
        box-shadow: 0 25px 50px rgba(0,0,0,0.5) !important;
        overflow: hidden;
    }

    .login-header {
        display: flex; flex-direction: column; align-items: center;
        gap: 0.75rem; text-align: center; padding: 1rem 0;
    }
    .logo-icon-big {
        width: 60px; height: 60px; border-radius: 16px;
        background: linear-gradient(135deg, rgba(99,0,255,0.3), rgba(0,200,150,0.2));
        border: 1px solid rgba(255,255,255,0.1);
        display: flex; align-items: center; justify-content: center;
    }
    .logo-icon-big .material-icons { font-size: 32px; color: #00c896; }
    .app-title { font-size: 2rem; font-weight: 900; color: #fff; margin: 0; letter-spacing: -0.03em; }
    .accent {
        background: linear-gradient(135deg, #00c896, #00a8ff);
        -webkit-background-clip: text; -webkit-text-fill-color: transparent; background-clip: text;
    }
    .app-subtitle { font-size: 0.875rem; color: rgba(255,255,255,0.42); max-width: 260px; line-height: 1.5; margin: 0; }

    .error-banner {
        display: flex; align-items: center; gap: 0.5rem;
        background: rgba(207,102,121,0.12); border: 1px solid rgba(207,102,121,0.28);
        border-radius: 10px; padding: 0.75rem 1rem; margin-top: 1rem;
        color: #cf6679; font-size: 0.875rem;
    }
    .error-icon { font-size: 18px; flex-shrink: 0; }

    :global(.login-actions) { padding: 0 1rem 1.25rem !important; }

    :global(.microsoft-btn) {
        width: 100% !important;
        background: linear-gradient(135deg, #2563eb, #1d4ed8) !important;
        color: #fff !important; border-radius: 12px !important; padding: 0.8rem !important;
        font-size: 0.95rem !important; font-weight: 600 !important;
        display: flex !important; align-items: center !important; justify-content: center !important;
        gap: 0.75rem !important; box-shadow: 0 4px 16px rgba(37,99,235,0.4) !important;
    }
    :global(.microsoft-btn:hover:not(:disabled)) {
        box-shadow: 0 6px 24px rgba(37,99,235,0.55) !important; transform: translateY(-1px);
    }
    :global(.microsoft-btn:disabled) { opacity: 0.7 !important; }

    .ms-logo { width: 20px; height: 20px; fill: currentColor; flex-shrink: 0; }

    .spinner {
        width: 18px; height: 18px;
        border: 2px solid rgba(255,255,255,0.3); border-top-color: #fff;
        border-radius: 50%; animation: spin 0.7s linear infinite; flex-shrink: 0;
    }

    .spinner-sm {
        width: 14px; height: 14px;
        border: 2px solid rgba(255,255,255,0.2); border-top-color: #00c896;
        border-radius: 50%; animation: spin 0.7s linear infinite; flex-shrink: 0;
    }

    @keyframes spin { to { transform: rotate(360deg); } }

    .pixelated { image-rendering: pixelated; image-rendering: crisp-edges; }
</style>