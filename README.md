# 🟢 Sodium Launcher

> A lightweight and personal Minecraft Java Edition launcher with Microsoft account authentication.

---

## 📖 About

**Sodium** is a custom Minecraft launcher built for personal use. It allows seamless authentication with a Microsoft account and launches Minecraft Java Edition directly, without the need for the official launcher.

---

## ✨ Features

- 🔐 Microsoft OAuth 2.0 authentication
- 🎮 Direct Minecraft Java Edition launch
- 🧹 Clean and minimal interface
- 💾 No sensitive data stored — tokens are session-only

---

## 🔧 How it works

1. The user logs in with their **Microsoft account** via OAuth 2.0
2. The launcher retrieves an **Xbox Live token**
3. It then authenticates with **Minecraft Services API** (`api.minecraftservices.com`)
4. The game is launched with the validated session

---

## 🛡️ Privacy & Security

- No personal data is collected or stored
- Authentication tokens are only used during the session for game login
- This project complies with **Microsoft's** and **Mojang's** Terms of Service

---

## 📦 Tech Stack

- Language: *(e.g. Java / Python / Electron — update as needed)*
- Auth: Microsoft OAuth 2.0
- APIs: Xbox Live API, Minecraft Services API

---

## 🚀 Usage

> This launcher is intended for **strictly personal use only**.

```bash
# Clone the repository
git clone https://github.com/tonpseudo/sodium-launcher

# Run the launcher
# (add your specific instructions here)
```

---

## 📄 License

This project is for personal use only and is not affiliated with Mojang or Microsoft.

---

*Made with ❤️ by EzRogue_*
