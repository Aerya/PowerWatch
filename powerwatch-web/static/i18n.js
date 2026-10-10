(() => {
  "use strict";

  const STORAGE_KEY = "powerwatch-language";
  const SUPPORTED = ["en", "fr"];

  const messages = {
    en: {
      "energy.title": "Energy consumption",
      "energy.estimate": "Partially estimated · not measured at the socket",
      "energy.24h": "Last 24 hours",
      "energy.7d": "Last 7 days",
      "energy.30d": "Last 30 days",
      "energy.all": "Since first recorded sample",
      "energy.from": "From",
      "energy.to": "To",
      "energy.apply": "Show period",
      "energy.reset": "Clear dates",
      "energy.selected": "Selected period",
      "energy.coverage": "Observed coverage",
      "energy.since": "Tracking since",
      "energy.noData": "No covered readings yet",
      "energy.invalidDates": "Choose valid start and end dates",
      "energy.loadError": "Energy unavailable",
      "page.dashboard": "PowerWatch",
      "page.alerts": "PowerWatch · Alerts",
      "nav.alerts": "Alerts",
      "nav.backDashboard": "Back to dashboard",
      "common.loading": "Loading...",
      "common.show": "Show",
      "common.cancel": "Cancel",
      "common.apply": "Apply",
      "common.close": "Close",
      "common.dismiss": "Dismiss",
      "common.reload": "Reload",
      "common.save": "Save",
      "common.delete": "Delete",
      "common.enabled": "Enabled",
      "common.recovery": "Recovery",
      "common.state": "State",
      "common.name": "Name",
      "common.component": "Component",
      "common.notify": "Notify",
      "common.runtime": "Runtime",
      "common.time": "Time",
      "common.watts": "Watts",
      "common.message": "Message",
      "common.action": "Action:",
      "common.token": "token",
      "dashboard.instanceName": "Instance",
      "dashboard.instanceSaved": "saved",
      "dashboard.instanceSaveFailed": "save failed",
      "dashboard.ramInstalled": "Installed RAM",
      "dashboard.ramUsable": "Usable RAM",
      "dashboard.powerSupply": "Power supply",
      "dashboard.memoryModule": "module",
      "dashboard.memoryModules": "modules",
      "dashboard.nominal": "nominal",
      "dashboard.psuUnverified": "SMBIOS unverified",
      "dashboard.usable": "usable",
      "dashboard.notReported": "not reported",
      "dashboard.notReportedSmbios": "not reported by SMBIOS",
      "dashboard.reportedBySmbios": "reported by SMBIOS",
      "dashboard.smbiosUnavailable": "SMBIOS unavailable",
      "dashboard.psuNotReported": "not exposed by SMBIOS firmware",
      "dashboard.moduleDetailsUnavailable": "memory-module details unavailable",
      "dashboard.table.component": "component",
      "dashboard.table.watts": "watts",
      "dashboard.table.confidence": "confidence",
      "dashboard.live": "Live",
      "dashboard.custom": "Custom:",
      "dashboard.hours": "hours",
      "dashboard.days": "days",
      "dashboard.weeks": "weeks",
      "dashboard.months": "months",
      "dashboard.years": "years",
      "dashboard.average": "Average",
      "dashboard.minimum": "Minimum",
      "dashboard.maximum": "Maximum",
      "dashboard.energy": "Energy",
      "dashboard.suggestions": "Energy Suggestions",
      "dashboard.applyEnergyAction": "Apply Energy Action",
      "dashboard.changeProfileHint": "This will change your system power profile.",
      "dashboard.actionApplied": "Action Applied",
      "dashboard.actionAppliedDefault": "The energy-saving action has been applied successfully.",
      "dashboard.actionFailed": "Action Failed",
      "dashboard.actionFailedDefault": "Failed to apply the action.",
      "dashboard.topProcessesTitle": "Top CPU Processes (Auto-refresh: 2s)",
      "dashboard.topProcessesHelp": "These are the processes consuming the most CPU right now:",
      "dashboard.stopRefresh": "⏹ Stop Refresh",
      "dashboard.autoRefresh": "▶ Auto Refresh",
      "dashboard.footerBasedOn": "Based on",
      "dashboard.footerThanks": "thank you for the original project.",
      "dashboard.totalNoData": "total: no data (all sensors failed)",
      "dashboard.serverUnreachable": "could not reach the server: {error}",
      "dashboard.alertsActive": "alerts: {active} active",
      "dashboard.alertsNormal": "alerts: normal",
      "dashboard.alertsUnavailable": "alerts: unavailable",
      "dashboard.enabledCount": "{count} enabled",
      "dashboard.confirmApply": "Are you sure you want to apply: \"{action}\"?",
      "dashboard.failedApply": "Failed to apply: {error}",
      "dashboard.error": "Error: {error}",
      "dashboard.failedFetch": "Failed to fetch: {status}",
      "dashboard.noOutput": "No output",
      "dashboard.loadingHistory": "loading {label} history…",
      "dashboard.historyError": "history error: {error}",
      "dashboard.live10m": "Live · 10 min",
      "dashboard.noHistory": "no history yet - waiting for data",
      "dashboard.allHidden": "every series is hidden - click the legend to show one",
      "dashboard.chartCaption": "{label} · {points} points · {resolution} resolution · updated {updated}",
      "confidence.measured": "measured",
      "confidence.estimated": "estimated",
      "severity.warning": "WARNING",
      "severity.info": "INFO",
      "suggestion.totalHigh": "Total power draw has been high. Consider switching to power saver profile.",
      "suggestion.cpuHigh": "CPU power draw has been high. Consider reducing workload or switching to power saver.",
      "suggestion.displayIdle": "System appears idle with display active. Consider enabling screensaver to save energy.",
      "suggestion.cpuProcesses": "CPU usage has been high for a while. Review top processes to identify what's consuming resources.",
      "suggestion.switchPowerSaver": "Switch to power saver",
      "suggestion.enableScreensaver": "Enable screensaver",
      "suggestion.showTopProcesses": "Show top processes",
      "alerts.notifications": "Notifications",
      "alerts.discordWebhook": "Discord webhook",
      "alerts.discordHelp": "Direct Discord webhook notification.",
      "alerts.appriseEndpoint": "Apprise API endpoint",
      "alerts.appriseHelp": "Use /notify/KEY for a saved Apprise config, or /notify/ for stateless mode.",
      "alerts.appriseUrls": "Apprise notification URLs (optional, one per line)",
      "alerts.appriseUrlsHelp": "Needed for stateless Apprise unless APPRISE_STATELESS_URLS is already configured.",
      "alerts.showSecrets": "Show webhook/API values",
      "alerts.testDiscord": "Test Discord",
      "alerts.testApprise": "Test Apprise",
      "alerts.testAll": "Test all configured",
      "alerts.rules": "Alert rules",
      "alerts.add": "Add alert",
      "alerts.threshold": "Threshold (W)",
      "alerts.forSeconds": "For (seconds)",
      "alerts.save": "Save settings",
      "alerts.recentEvents": "Recent events",
      "alerts.defaultRuleName": "Power alert",
      "alerts.noEvents": "No alert events yet.",
      "alerts.notSaved": "not saved",
      "alerts.noData": "no data",
      "alerts.waiting": "waiting · {seconds}s",
      "alerts.active": "ALERT · {watts} W",
      "alerts.ok": "ok · {watts} W",
      "alerts.loading": "Loading...",
      "alerts.loadFailed": "Failed to load alerts: {error}",
      "alerts.saving": "Saving...",
      "alerts.saved": "Settings saved.",
      "alerts.saveFailed": "Save failed: {error}",
      "alerts.testing": "Testing {channel}...",
      "alerts.testFailed": "Test failed: {error}",
      "alerts.failed": "FAILED — {error}",
      "alerts.unknownError": "unknown error",
      "alerts.triggered": "triggered",
      "alerts.recovered": "recovered",
      "alerts.eventTriggered": "{rule}: {component} is {watts} W (threshold {threshold} W for {seconds}s)",
      "alerts.eventRecovered": "{rule} recovered: {component} is {watts} W (threshold {threshold} W)"
    },
    fr: {
      "energy.title": "Consommation énergétique cumulée",
      "energy.estimate": "Partiellement estimée · non mesurée à la prise",
      "energy.24h": "Dernières 24 heures",
      "energy.7d": "7 derniers jours",
      "energy.30d": "30 derniers jours",
      "energy.all": "Depuis le premier relevé",
      "energy.from": "Du",
      "energy.to": "Au",
      "energy.apply": "Afficher la période",
      "energy.reset": "Effacer les dates",
      "energy.selected": "Période sélectionnée",
      "energy.coverage": "Durée réellement observée",
      "energy.since": "Suivi depuis",
      "energy.noData": "Aucun relevé exploitable",
      "energy.invalidDates": "Choisir des dates de début et de fin valides",
      "energy.loadError": "Consommation indisponible",
      "page.dashboard": "PowerWatch",
      "page.alerts": "PowerWatch · Alertes",
      "nav.alerts": "Alertes",
      "nav.backDashboard": "Retour au tableau de bord",
      "common.loading": "Chargement...",
      "common.show": "Afficher",
      "common.cancel": "Annuler",
      "common.apply": "Appliquer",
      "common.close": "Fermer",
      "common.dismiss": "Fermer",
      "common.reload": "Recharger",
      "common.save": "Enregistrer",
      "common.delete": "Supprimer",
      "common.enabled": "Activée",
      "common.recovery": "Retour à la normale",
      "common.state": "État",
      "common.name": "Nom",
      "common.component": "Composant",
      "common.notify": "Notification",
      "common.runtime": "État actuel",
      "common.time": "Date",
      "common.watts": "Watts",
      "common.message": "Message",
      "common.action": "Action :",
      "common.token": "jeton",
      "dashboard.instanceName": "Instance",
      "dashboard.instanceSaved": "enregistré",
      "dashboard.instanceSaveFailed": "échec de l’enregistrement",
      "dashboard.ramInstalled": "RAM installée",
      "dashboard.ramUsable": "RAM utilisable",
      "dashboard.powerSupply": "Alimentation",
      "dashboard.memoryModule": "module",
      "dashboard.memoryModules": "modules",
      "dashboard.nominal": "nominale",
      "dashboard.psuUnverified": "SMBIOS non vérifié",
      "dashboard.usable": "utilisables",
      "dashboard.notReported": "non reportée",
      "dashboard.notReportedSmbios": "non reportée par SMBIOS",
      "dashboard.reportedBySmbios": "reportée par SMBIOS",
      "dashboard.smbiosUnavailable": "SMBIOS inaccessible",
      "dashboard.psuNotReported": "non fournie par le firmware SMBIOS",
      "dashboard.moduleDetailsUnavailable": "détails des modules mémoire indisponibles",
      "dashboard.table.component": "composant",
      "dashboard.table.watts": "watts",
      "dashboard.table.confidence": "mesure",
      "dashboard.live": "Direct",
      "dashboard.custom": "Personnalisé :",
      "dashboard.hours": "heures",
      "dashboard.days": "jours",
      "dashboard.weeks": "semaines",
      "dashboard.months": "mois",
      "dashboard.years": "années",
      "dashboard.average": "Moyenne",
      "dashboard.minimum": "Minimum",
      "dashboard.maximum": "Maximum",
      "dashboard.energy": "Énergie",
      "dashboard.suggestions": "Suggestions d’économie d’énergie",
      "dashboard.applyEnergyAction": "Appliquer l’action d’économie d’énergie",
      "dashboard.changeProfileHint": "Cette action modifiera le profil énergétique du système.",
      "dashboard.actionApplied": "Action appliquée",
      "dashboard.actionAppliedDefault": "L’action d’économie d’énergie a été appliquée avec succès.",
      "dashboard.actionFailed": "Échec de l’action",
      "dashboard.actionFailedDefault": "Impossible d’appliquer l’action.",
      "dashboard.topProcessesTitle": "Processus CPU les plus actifs (actualisation : 2 s)",
      "dashboard.topProcessesHelp": "Voici les processus qui utilisent actuellement le plus le CPU :",
      "dashboard.stopRefresh": "⏹ Arrêter l’actualisation",
      "dashboard.autoRefresh": "▶ Actualisation auto",
      "dashboard.footerBasedOn": "Basé sur",
      "dashboard.footerThanks": "merci pour le projet d’origine.",
      "dashboard.totalNoData": "total : aucune donnée (tous les capteurs ont échoué)",
      "dashboard.serverUnreachable": "serveur inaccessible : {error}",
      "dashboard.alertsActive": "alertes : {active} active(s)",
      "dashboard.alertsNormal": "alertes : normal",
      "dashboard.alertsUnavailable": "alertes : indisponibles",
      "dashboard.enabledCount": "{count} activée(s)",
      "dashboard.confirmApply": "Voulez-vous vraiment appliquer : \"{action}\" ?",
      "dashboard.failedApply": "Échec de l’application : {error}",
      "dashboard.error": "Erreur : {error}",
      "dashboard.failedFetch": "Échec de la récupération : {status}",
      "dashboard.noOutput": "Aucun résultat",
      "dashboard.loadingHistory": "chargement de l’historique {label}…",
      "dashboard.historyError": "erreur d’historique : {error}",
      "dashboard.live10m": "Direct · 10 min",
      "dashboard.noHistory": "aucun historique pour le moment - en attente de données",
      "dashboard.allHidden": "toutes les séries sont masquées - cliquez sur la légende pour en afficher une",
      "dashboard.chartCaption": "{label} · {points} points · résolution {resolution} · mis à jour à {updated}",
      "confidence.measured": "mesuré",
      "confidence.estimated": "estimé",
      "severity.warning": "ALERTE",
      "severity.info": "INFO",
      "suggestion.totalHigh": "La consommation totale reste élevée. Envisagez de passer en mode économie d’énergie.",
      "suggestion.cpuHigh": "La consommation CPU reste élevée. Envisagez de réduire la charge ou de passer en mode économie d’énergie.",
      "suggestion.displayIdle": "Le système semble inactif alors que l’écran est actif. Activez l’économiseur d’écran pour réduire la consommation.",
      "suggestion.cpuProcesses": "L’utilisation CPU reste élevée. Consultez les processus les plus actifs pour identifier ce qui consomme des ressources.",
      "suggestion.switchPowerSaver": "Passer en mode économie d’énergie",
      "suggestion.enableScreensaver": "Activer l’économiseur d’écran",
      "suggestion.showTopProcesses": "Afficher les processus les plus actifs",
      "alerts.notifications": "Notifications",
      "alerts.discordWebhook": "Webhook Discord",
      "alerts.discordHelp": "Notification directe via un webhook Discord.",
      "alerts.appriseEndpoint": "Point d’accès API Apprise",
      "alerts.appriseHelp": "Utilisez /notify/KEY pour une configuration Apprise enregistrée, ou /notify/ en mode stateless.",
      "alerts.appriseUrls": "URLs de notification Apprise (facultatif, une par ligne)",
      "alerts.appriseUrlsHelp": "Nécessaire en mode Apprise stateless sauf si APPRISE_STATELESS_URLS est déjà configuré.",
      "alerts.showSecrets": "Afficher les valeurs webhook/API",
      "alerts.testDiscord": "Tester Discord",
      "alerts.testApprise": "Tester Apprise",
      "alerts.testAll": "Tester toutes les notifications configurées",
      "alerts.rules": "Règles d’alerte",
      "alerts.add": "Ajouter une alerte",
      "alerts.threshold": "Seuil (W)",
      "alerts.forSeconds": "Durée (secondes)",
      "alerts.save": "Enregistrer",
      "alerts.recentEvents": "Événements récents",
      "alerts.defaultRuleName": "Alerte de consommation",
      "alerts.noEvents": "Aucun événement d’alerte pour le moment.",
      "alerts.notSaved": "non enregistrée",
      "alerts.noData": "aucune donnée",
      "alerts.waiting": "en attente · {seconds}s",
      "alerts.active": "ALERTE · {watts} W",
      "alerts.ok": "ok · {watts} W",
      "alerts.loading": "Chargement...",
      "alerts.loadFailed": "Impossible de charger les alertes : {error}",
      "alerts.saving": "Enregistrement...",
      "alerts.saved": "Paramètres enregistrés.",
      "alerts.saveFailed": "Échec de l’enregistrement : {error}",
      "alerts.testing": "Test de {channel}...",
      "alerts.testFailed": "Échec du test : {error}",
      "alerts.failed": "ÉCHEC — {error}",
      "alerts.unknownError": "erreur inconnue",
      "alerts.triggered": "déclenchée",
      "alerts.recovered": "rétablie",
      "alerts.eventTriggered": "{rule} : {component} est à {watts} W (seuil {threshold} W pendant {seconds}s)",
      "alerts.eventRecovered": "{rule} rétablie : {component} est à {watts} W (seuil {threshold} W)"
    }
  };

  const suggestionKeys = {
    "Total power draw has been high. Consider switching to power saver profile.": "suggestion.totalHigh",
    "CPU power draw has been high. Consider reducing workload or switching to power saver.": "suggestion.cpuHigh",
    "System appears idle with display active. Consider enabling screensaver to save energy.": "suggestion.displayIdle",
    "CPU usage has been high for a while. Review top processes to identify what's consuming resources.": "suggestion.cpuProcesses",
    "Switch to power saver": "suggestion.switchPowerSaver",
    "Enable screensaver": "suggestion.enableScreensaver",
    "Show top processes": "suggestion.showTopProcesses"
  };

  function detectLanguage() {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (SUPPORTED.includes(saved)) return saved;
    return String(navigator.language || "en").toLowerCase().startsWith("fr") ? "fr" : "en";
  }

  let currentLanguage = detectLanguage();

  function t(key, vars = {}) {
    let value = messages[currentLanguage]?.[key] ?? messages.en[key] ?? key;
    for (const [name, replacement] of Object.entries(vars)) {
      value = value.replaceAll(`{${name}}`, String(replacement));
    }
    return value;
  }

  function apply(root = document) {
    document.documentElement.lang = currentLanguage;
    document.title = t(document.body?.dataset?.page === "alerts" ? "page.alerts" : "page.dashboard");
    root.querySelectorAll("[data-i18n]").forEach((el) => {
      el.textContent = t(el.dataset.i18n);
    });
    root.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
      el.placeholder = t(el.dataset.i18nPlaceholder);
    });
    document.querySelectorAll("[data-lang]").forEach((button) => {
      const active = button.dataset.lang === currentLanguage;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", active ? "true" : "false");
    });
  }

  function setLanguage(language) {
    if (!SUPPORTED.includes(language)) return;
    currentLanguage = language;
    localStorage.setItem(STORAGE_KEY, language);
    apply();
    window.dispatchEvent(new CustomEvent("powerwatch:languagechange", { detail: { language } }));
  }

  function confidence(value) {
    return value === "Measured" ? t("confidence.measured") : t("confidence.estimated");
  }

  function suggestion(value) {
    const key = suggestionKeys[value];
    return key ? t(key) : value;
  }

  function rangeLabel(amount, unit) {
    const unitKey = {
      hours: "dashboard.hours",
      days: "dashboard.days",
      weeks: "dashboard.weeks",
      months: "dashboard.months",
      years: "dashboard.years"
    }[unit];
    return unitKey ? `${amount} ${t(unitKey)}` : `${amount} ${unit}`;
  }

  function alertEventMessage(event) {
    const vars = {
      rule: event.rule_name,
      component: event.component,
      watts: Number(event.watts).toFixed(1),
      threshold: Number(event.threshold_watts).toFixed(1),
      seconds: event.sustained_seconds ?? "?"
    };
    if (event.kind === "recovered") return t("alerts.eventRecovered", vars);
    if (event.kind === "triggered" && event.sustained_seconds != null) return t("alerts.eventTriggered", vars);
    return event.message;
  }

  window.PWI18n = {
    t,
    apply,
    setLanguage,
    confidence,
    suggestion,
    rangeLabel,
    alertEventMessage,
    get language() { return currentLanguage; }
  };
  window.t = t;
  window.setPowerWatchLanguage = setLanguage;

  document.addEventListener("DOMContentLoaded", () => apply());
})();
