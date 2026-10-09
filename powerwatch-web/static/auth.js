(() => {
  "use strict";

  const nativeFetch = window.fetch.bind(window);
  let csrfToken = null;
  let authEnabled = false;
  let resolveReady;
  const authReady = new Promise(resolve => { resolveReady = resolve; });
  window.PowerWatchAuth = { ready: authReady, enabled: false };

  function language() {
    const saved = localStorage.getItem("powerwatch-language");
    if (saved === "fr" || saved === "en") return saved;
    return navigator.language?.toLowerCase().startsWith("fr") ? "fr" : "en";
  }

  const copy = {
    fr: {
      setupTitle: "Créer le compte administrateur",
      setupHelp: "L’authentification PowerWatch est activée. Saisissez le jeton d’initialisation configuré sur le serveur, puis créez l’unique compte administrateur.",
      setupToken: "Jeton d’initialisation",
      username: "Identifiant",
      password: "Mot de passe (12 caractères minimum)",
      confirm: "Confirmer le mot de passe",
      create: "Créer le compte",
      loginTitle: "Connexion à PowerWatch",
      loginHelp: "Connectez-vous avec l’unique compte administrateur.",
      login: "Se connecter",
      mismatch: "Les mots de passe ne correspondent pas.",
      unavailable: "Impossible de contacter le service d’authentification.",
      security: "Sécurité",
      logout: "Déconnexion"
    },
    en: {
      setupTitle: "Create the administrator account",
      setupHelp: "PowerWatch authentication is enabled. Enter the setup token configured on the server, then create the single administrator account.",
      setupToken: "Setup token",
      username: "Username",
      password: "Password (12 characters minimum)",
      confirm: "Confirm password",
      create: "Create account",
      loginTitle: "Sign in to PowerWatch",
      loginHelp: "Sign in with the single administrator account.",
      login: "Sign in",
      mismatch: "Passwords do not match.",
      unavailable: "The authentication service is unavailable.",
      security: "Security",
      logout: "Sign out"
    }
  };

  function message(error, fallback) {
    if (!error) return fallback;
    try { return JSON.parse(error).error || fallback; } catch (_) { return error || fallback; }
  }

  function injectStyles() {
    const style = document.createElement("style");
    style.textContent = `
      .pw-auth-gate{position:fixed;inset:0;z-index:10000;display:grid;place-items:center;padding:20px;background:#0d1117;color:#c9d1d9;font-family:ui-monospace,"Cascadia Code","Fira Code",monospace}
      .pw-auth-card{width:min(460px,100%);background:#161b22;border:1px solid #30363d;border-radius:8px;padding:24px;box-shadow:0 18px 60px rgba(0,0,0,.45)}
      .pw-auth-card h1{font-size:1.15rem;margin:0 0 8px}.pw-auth-card p{color:#8b949e;font-size:.82rem;line-height:1.55;margin:0 0 18px}
      .pw-auth-card label{display:block;color:#8b949e;font-size:.78rem;margin:12px 0 5px}.pw-auth-card input{width:100%;font:inherit;color:#c9d1d9;background:#0d1117;border:1px solid #30363d;border-radius:4px;padding:9px 10px}
      .pw-auth-card input:focus{outline:none;border-color:#3fb950}.pw-auth-card button,.pw-auth-control{font:inherit;color:#c9d1d9;background:transparent;border:1px solid #30363d;border-radius:4px;padding:7px 10px;cursor:pointer}
      .pw-auth-card button{width:100%;margin-top:18px;color:#3fb950;border-color:#3fb950}.pw-auth-card button:disabled{opacity:.55;cursor:wait}
      .pw-auth-error{min-height:20px;color:#f85149;font-size:.78rem;margin-top:10px;white-space:pre-wrap}.pw-auth-controls{display:flex;gap:6px;align-items:center}.pw-auth-controls a{color:#c9d1d9;text-decoration:none;border-bottom:1px dotted #8b949e;font-size:.78rem}
      .pw-auth-control{font-size:.78rem;padding:4px 7px}.pw-auth-brand{color:#3fb950;font-weight:700;margin-bottom:14px}
    `;
    document.head.appendChild(style);
  }

  function showGate(status) {
    const lang = language();
    const text = copy[lang];
    const setup = status.setup_required;
    const gate = document.createElement("div");
    gate.className = "pw-auth-gate";
    gate.innerHTML = `<form class="pw-auth-card" autocomplete="on">
      <div class="pw-auth-brand">PowerWatch</div>
      <h1>${setup ? text.setupTitle : text.loginTitle}</h1>
      <p>${setup ? text.setupHelp : text.loginHelp}</p>
      ${setup ? `<label for="pw-setup-token">${text.setupToken}</label><input id="pw-setup-token" name="setup-token" type="password" autocomplete="off" required>` : ""}
      <label for="pw-username">${text.username}</label><input id="pw-username" name="username" autocomplete="username" maxlength="64" required autofocus>
      <label for="pw-password">${text.password}</label><input id="pw-password" name="password" type="password" autocomplete="${setup ? "new-password" : "current-password"}" minlength="12" required>
      ${setup ? `<label for="pw-password-confirm">${text.confirm}</label><input id="pw-password-confirm" name="password-confirm" type="password" autocomplete="new-password" minlength="12" required>` : ""}
      <button type="submit">${setup ? text.create : text.login}</button>
      <div class="pw-auth-error" role="alert"></div>
    </form>`;
    document.body.appendChild(gate);
    const form = gate.querySelector("form");
    form.addEventListener("submit", async event => {
      event.preventDefault();
      const error = gate.querySelector(".pw-auth-error");
      const button = gate.querySelector("button");
      const password = gate.querySelector("#pw-password").value;
      if (setup && password !== gate.querySelector("#pw-password-confirm").value) {
        error.textContent = text.mismatch;
        return;
      }
      button.disabled = true;
      error.textContent = "";
      const payload = { username: gate.querySelector("#pw-username").value, password };
      if (setup) payload.setup_token = gate.querySelector("#pw-setup-token").value;
      try {
        const response = await nativeFetch(setup ? "/api/auth/setup" : "/api/auth/login", {
          method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload)
        });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json();
        csrfToken = result.csrf_token;
        gate.remove();
        addControls(result.username);
        resolveReady();
      } catch (failure) {
        error.textContent = message(failure.message, text.unavailable);
        button.disabled = false;
      }
    });
  }

  function addControls() {
    if (!authEnabled || document.querySelector(".pw-auth-controls")) return;
    const text = copy[language()];
    const controls = document.createElement("div");
    controls.className = "pw-auth-controls";
    controls.innerHTML = `<a href="/security">${text.security}</a><button class="pw-auth-control" type="button">${text.logout}</button>`;
    controls.querySelector("button").addEventListener("click", async () => {
      await window.fetch("/api/auth/logout", { method: "POST" });
      location.assign("/");
    });
    const switcher = document.querySelector(".language-switcher");
    if (switcher?.parentNode) switcher.parentNode.insertBefore(controls, switcher.nextSibling);
    else document.body.prepend(controls);
  }

  window.fetch = async (input, init = {}) => {
    const requestUrl = typeof input === "string" ? input : input.url;
    const url = new URL(requestUrl, location.href);
    const publicAuth = ["/api/auth/status", "/api/auth/login", "/api/auth/setup"].includes(url.pathname);
    if (url.origin === location.origin && !publicAuth) await authReady;
    const options = { ...init };
    const method = String(options.method || (typeof input !== "string" && input.method) || "GET").toUpperCase();
    if (url.origin === location.origin && csrfToken && !["GET", "HEAD", "OPTIONS"].includes(method)) {
      const headers = new Headers(options.headers || (typeof input !== "string" ? input.headers : undefined));
      headers.set("X-CSRF-Token", csrfToken);
      options.headers = headers;
    }
    return nativeFetch(input, options);
  };

  async function initialize() {
    injectStyles();
    try {
      const response = await nativeFetch("/api/auth/status", { cache: "no-store" });
      if (!response.ok) throw new Error(await response.text());
      const status = await response.json();
      authEnabled = status.enabled;
      window.PowerWatchAuth.enabled = status.enabled;
      if (!status.enabled) {
        resolveReady();
      } else if (status.authenticated) {
        csrfToken = status.csrf_token;
        addControls(status.username);
        resolveReady();
      } else {
        showGate(status);
      }
    } catch (_) {
      showGate({ setup_required: false });
      document.querySelector(".pw-auth-error").textContent = copy[language()].unavailable;
    }
  }

  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", initialize);
  else initialize();
})();
