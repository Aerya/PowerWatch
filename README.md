# PowerWatch
<p align="center">
  <img src="docs/images/powerwatch-aerya-logo.png" alt="PowerWatch" width="720">
</p>

<p align="center">
  <strong>Français</strong> · <a href="README.en.md">English</a>
</p>

<p align="center">
  <img src="docs/images/hubminiEN.png" alt="Aperçu de PowerWatch Hub avec trois machines fédérées" width="1200">
</p>

<p align="center">
  <em>PowerWatch Hub centralise la consommation de plusieurs machines dans un seul dashboard.</em>
</p>

## Sommaire

- [Fonctionnalités](#fonctionnalités)
- [Mesures](#mesures)
  - [Cartes ARM et couverture des mesures](#cartes-arm-et-couverture-des-mesures)
  - [RAM et alimentation](#ram-et-alimentation)
- [Installation Docker](#installation-docker)
  - [Prérequis](#prérequis)
  - [Démarrage rapide](#démarrage-rapide)
  - [CPU RAPL via MSR](#cpu-rapl-via-msr--synology-dsm-et-noyaux-sans-powercap)
- [Compilation native](#compilation-native)
- [GPU](#gpu)
  - [AMD](#amd)
  - [Intel](#intel)
  - [NVIDIA](#nvidia)
- [Disques, partitions et stockage](#disques-partitions-et-stockage)
- [WebUI](#webui)
  - [Tarification électrique France — bêta (optionnelle)](#tarification-électrique-france--bêta-optionnelle)
- [Consommation énergétique cumulée (kWh)](#consommation-énergétique-cumulée-kwh)
- [Historique](#historique)
- [Alertes et notifications](#alertes-et-notifications)
- [Authentification intégrée](#authentification-intégrée)
- [CLI et TUI dans Docker](#cli-et-tui-dans-docker)
- [Vérifier les capteurs détectés](#vérifier-les-capteurs-détectés)
- [PowerWatch Hub](#powerwatch-hub--plusieurs-machines-un-seul-dashboard)
  - [Sélection des instances pour les cumuls](#sélection-des-instances-pour-les-cumuls)
  - [Démarrage du Hub](#démarrage-du-hub)
- [Home Assistant — découverte MQTT](#home-assistant--découverte-mqtt-facultative)
- [Sécurité](#sécurité)
- [Attribution](#attribution)
- [Licence](#licence)

**PowerWatch** surveille la consommation électrique d'une machine Linux depuis Docker et l'affiche dans une WebUI, avec historique, alertes et plusieurs sources matérielles réelles ou estimées.

Ce fork est pensé en priorité pour les **serveurs, mini-PC, machines desktop Linux et hôtes Docker**. **Docker reste la méthode de déploiement recommandée et la mieux maîtrisée** ; une compilation native depuis les sources est également documentée pour les utilisateurs avancés.

> **Sécurité :** l'authentification intégrée est facultative et désactivée par défaut. Sans elle, utilisez la WebUI uniquement sur un **LAN privé de confiance**.

## Fonctionnalités

- WebUI temps réel avec vue historique.
- Historique SQLite persistant avec agrégation longue durée.
- Alertes configurables depuis la WebUI.
- Notifications **Discord** et **Apprise API**.
- Détection multi-GPU **NVIDIA / AMD / Intel**.
- Fallback **Intel RAPL `uncore`** pour certains iGPU sans compteur i915/xe hwmon.
- Fallback CPU **RAPL via `/dev/cpu/0/msr`** lorsque le noyau n'expose pas `powercap` (notamment certains Synology DSM).
- Suivi de tous les **disques physiques** sans double comptage RAID/LVM, avec affichage de leur capacité.
- Affichage best-effort des **partitions montées, systèmes de fichiers, mdraid, LVM et dm-crypt** liés aux disques, sans bruit provenant des partitions non montées.
- CLI et TUI disponibles dans l'image Docker.
- Images GHCR multi-architecture **amd64 / arm64**.
- Cartes ARM : modèle de puissance de la carte calibré à la prise ; sur Raspberry Pi 5, mesure des rails PMIC lorsque `/dev/vcio_gencmd` est accessible.
- Interface Web **français / anglais**.
- Authentification WebUI facultative avec compte administrateur unique, sessions persistantes et jetons API en lecture seule.
- Nom d'instance PowerWatch configurable dans la WebUI et proposé automatiquement lors de l'ajout dans le Hub, sans empêcher un alias Hub différent.
- Modèle CPU affiché avec lien direct vers la recherche **CPU Benchmark / PassMark** et icônes monochromes CPU/RAM/disque.
- Inventaire RAM best-effort : **capacité installée, nombre de modules occupés et détails de chaque module** via SMBIOS lorsque disponibles.
- Informations alimentation best-effort via **SMBIOS Type 39** : puissance nominale maximale, fabricant/modèle, emplacement, type et état lorsque le firmware les publie.
- **PowerWatch Hub** : agrégation de plusieurs machines dans un dashboard fédéré unique.
- **Home Assistant (MQTT Discovery)** : capteurs W/kWh et coûts estimés en € automatiques depuis une instance ou le Hub, facultatif.

## Mesures

### Cartes ARM et couverture des mesures

Sur Linux ARM64 ou ARMv7, `POWERWATCH_BOARD_IDLE_W` et `POWERWATCH_BOARD_MAX_W` activent un modèle **Board** calibré à la prise. Mesurez la puissance de la carte **et des périphériques alimentés par la même prise** au repos stable, puis avec tous les cœurs CPU durablement chargés. Utilisez ces deux mesures en watts pour les variables : `Board = repos + (maximum − repos) × utilisation CPU`. Aucune valeur n'est déduite automatiquement. La lecture reste **estimée** et ne remplace pas RAPL/MSR lorsqu'ils existent. La calibration inclut RAM, stockage et pertes présents pendant la mesure ; ces composants ne sont pas ajoutés une seconde fois. Une charge périphérique qui varie indépendamment du CPU peut échapper à ce modèle.

Dans `compose.yaml`, ajoutez sous `services.powerwatch.environment` les deux valeurs relevées, par exemple `POWERWATCH_BOARD_IDLE_W: ${POWERWATCH_BOARD_IDLE_W:-}` et `POWERWATCH_BOARD_MAX_W: ${POWERWATCH_BOARD_MAX_W:-}`, puis définissez-les dans `.env`. Ces deux nombres sont propres à **votre** installation : les valeurs mesurées sur une autre carte ne conviennent pas. Si les deux valeurs sont absentes, le modèle reste inactif ; si une seule manque ou si une valeur est invalide, l'interface signale l'erreur du capteur.

Sur Raspberry Pi 5, le PMIC lit les rails via `/dev/vcio_gencmd` (groupe `video`). Pour Docker, ajoutez sous `services.powerwatch` :

```yaml
devices:
  - /dev/vcio_gencmd:/dev/vcio_gencmd
```

Vérifiez que le périphérique existe sur l'hôte et que le processus peut l'ouvrir ; en exécution native, l'utilisateur doit généralement appartenir au groupe `video`. Le conteneur officiel tourne en root et n'exige pas `privileged: true`. Si l'identité de la carte est masquée dans le conteneur, exposez aussi `/proc/device-tree` en lecture seule pour que les nouvelles tentatives après un premier échec reconnaissent le Pi 5. Le PMIC mesure les rails, mais ne couvre pas les périphériques alimentés directement en 5 V (USB, PCIe/NVMe, ventilateur) ni les pertes de l'alimentation. Le total, les kWh et les euros fondés sur cette seule source sont donc **partiels, en borne basse**, sans extrapolation des watts manquants. Les estimations USB/NVMe peuvent s'ajouter ; RAM et carte SD ne sont pas ajoutées deux fois. Si le modèle calibré est configuré, il prend la priorité sur le PMIC.

Les images publiées sont amd64 et arm64. Le binaire ARMv7 a été compilé et essayé en natif sur Raspberry Pi 2 B ; le modèle a aussi été essayé sur Pi 3 A+, Pi 4 B et Pi 5. Aucune image Docker ARMv7 n'est publiée.


| Composant | Source | Type |
|---|---|---|
| CPU | RAPL Linux (`powercap` ou MSR `/dev/cpu/0/msr`) | Mesurée |
| Carte ARM (modèle) | Charge CPU et calibration à la prise | Estimée |
| Raspberry Pi 5 (PMIC) | Rails du PMIC via `/dev/vcio_gencmd` | Mesurée, partielle |
| GPU NVIDIA | NVML | Mesurée |
| GPU AMD | `amdgpu` hwmon | Mesurée |
| GPU Intel | `i915` / `xe` hwmon ou RAPL `uncore` | Mesurée |
| RAM | Heuristique | Estimée |
| Disques | Temps occupé (`/proc/diskstats`) + profil du média ; power states NVMe si disponibles | Estimée |
| Total | Somme des capteurs disponibles | Mixte |

Un capteur indisponible est simplement ignoré.

Sur certains Intel, PowerWatch peut utiliser le sous-domaine RAPL `uncore` comme mesure de l'iGPU. Dans ce cas, la valeur CPU est calculée à partir du package en retirant `uncore` afin de ne pas compter deux fois l'iGPU.

Lorsque l'interface Linux `powercap` est absente mais que `/dev/cpu/0/msr` existe, PowerWatch peut lire directement les registres RAPL `MSR_RAPL_POWER_UNIT` (`0x606`) et `MSR_PKG_ENERGY_STATUS` (`0x611`). Cette valeur reste une **mesure matérielle** du package CPU, pas une estimation basée sur le pourcentage d'utilisation.

### RAM et alimentation

PowerWatch complète les mesures électriques avec un petit inventaire matériel best-effort, affiché dans la WebUI locale et transmis au Hub.

Pour la **RAM** :

- la mémoire totale utilisable est lue depuis `/proc/meminfo` ;
- PowerWatch lit en priorité la table SMBIOS/DMI brute du noyau pour récupérer la capacité réellement installée et les périphériques mémoire occupés ;
- l'interface **regroupe les modules identiques** en un résumé court (ex. `2 × 16 GiB · DDR4 · 3200 MT/s`) sans afficher les longues références constructeur ou les emplacements ; les métadonnées complètes restent disponibles dans l'API ;
- le nombre affiché correspond aux **Memory Devices SMBIOS occupés**. Sur certaines machines, de la mémoire soudée peut donc apparaître comme un module même s'il ne s'agit pas physiquement d'une barrette amovible ;
- si SMBIOS est inaccessible, PowerWatch affiche la mémoire **utilisable** vue par Linux et ne la présente plus comme de la « RAM installée ».

Pour l'**alimentation** :

- PowerWatch lit SMBIOS **Type 39 / System Power Supply** ;
- les entrées génériques du firmware (`Default string`, `OEM Define`, états/types sans capacité ni identité crédible) sont masquées, ainsi que les baies explicitement absentes ;
- la WebUI et le Hub n'affichent qu'un **résumé compact**, sans répéter les watts dans la même ligne ; les détails éventuels restent accessibles en infobulle ;
- une puissance annoncée est explicitement marquée **« SMBIOS non vérifié »** : même une valeur apparemment plausible peut différer de la puissance réelle du bloc d'alimentation ;
- la valeur en watts est une **capacité nominale maximale déclarée via SMBIOS, non vérifiée physiquement**, pas sa consommation électrique instantanée ;
- cette puissance nominale n'est **jamais ajoutée** au total de consommation PowerWatch.

Docker masque normalement `/sys/firmware` dans les conteneurs. Le Compose PowerWatch monte donc explicitement le firmware de l'hôte en lecture seule sous `/host-sys-firmware` et définit `POWERWATCH_DMI_TABLE_PATH=/host-sys-firmware/dmi/tables/DMI`. PowerWatch parse directement cette table : aucun mode `privileged` ni accès à `/dev/mem` n'est nécessaire.

De nombreuses cartes mères grand public et alimentations ATX classiques ne publient aucun SMBIOS Type 39. PowerWatch distingue alors deux cas : **SMBIOS inaccessible** ou **alimentation non fournie par le firmware SMBIOS**, au lieu de laisser croire que les deux situations sont identiques. `dmidecode` reste disponible comme fallback lorsque la table brute n'est pas lisible.

## Installation Docker

### Prérequis

- Linux ;
- Docker Engine ;
- Docker Compose ;
- accès en lecture à `/sys` depuis le conteneur ;
- pour NVIDIA : pilote NVIDIA + NVIDIA Container Toolkit sur l'hôte.

### Démarrage rapide

```bash
git clone https://github.com/Aerya/PowerWatch.git
cd PowerWatch

cp .env.example .env
docker compose pull
docker compose up -d
```

Par défaut, le Compose publie la WebUI uniquement sur :

```text
127.0.0.1:3000
```

Pour l'ouvrir sur le réseau local, modifiez `.env` avec **l'adresse IPv4 privée du serveur** :

```env
POWERWATCH_BIND_IP=192.168.0.50
```

Puis relancez :

```bash
docker compose up -d
```

La WebUI sera alors disponible sur :

```text
http://192.168.0.50:3000
```

Le fichier [`compose.yaml`](compose.yaml) fourni est le fichier de déploiement de référence.

### CPU RAPL via MSR — Synology DSM et noyaux sans `powercap`

Sur certains systèmes Intel, notamment certains **Synology DSM**, le noyau n'expose pas `/sys/.../powercap` alors que le périphérique `/dev/cpu/0/msr` est disponible.

Dans ce cas, décommentez dans `compose.yaml` les deux blocs suivants :

```yaml
cap_add:
  - SYS_RAWIO

devices:
  - /dev/cpu/0/msr:/dev/cpu/0/msr:r
```

Le pilote Linux `msr` exige **`CAP_SYS_RAWIO`** pour ouvrir `/dev/cpu/*/msr`, y compris lorsque le processus du conteneur tourne en root. Il n'est pas nécessaire d'utiliser `privileged: true` : n'activez que `SYS_RAWIO` avec le device MSR lorsque ce fallback est réellement nécessaire.

PowerWatch utilise toujours `powercap` en priorité. Le MSR n'est utilisé qu'en fallback si aucun domaine RAPL `powercap` n'est découvert. Un seul device CPU est nécessaire : le compteur `MSR_PKG_ENERGY_STATUS` est **commun au package**, il ne faut donc pas additionner `/dev/cpu/0/msr` et `/dev/cpu/1/msr`.

Pour les détails matériels et Docker : [`DOCKER.md`](DOCKER.md).

## Compilation native

Docker reste la méthode de déploiement recommandée pour PowerWatch. La compilation native est destinée aux utilisateurs avancés qui souhaitent exécuter directement les binaires sur leur distribution Linux.

### Prérequis

Il faut disposer de :

- **Git** ;
- une version stable récente de **Rust** et **Cargo** ;
- une chaîne de compilation C fonctionnelle (`gcc` ou `clang`, linker, `make`, etc.).

Rust peut être installé avec la méthode officielle [`rustup`](https://rustup.rs/).

Exemples de prérequis système courants :

```bash
# Debian / Ubuntu
sudo apt install build-essential pkg-config git curl dmidecode

# Fedora
sudo dnf group install "Development Tools"
sudo dnf install git pkgconf-pkg-config curl dmidecode

# Arch Linux / Garuda
sudo pacman -S --needed base-devel git curl dmidecode
```

### Compiler PowerWatch

Clonez le dépôt puis compilez tout le workspace en mode release :

```bash
git clone https://github.com/Aerya/PowerWatch.git
cd PowerWatch
cargo build --release --workspace
```

Les quatre exécutables sont générés dans `target/release/` :

```text
powerwatch
powerwatch-tui
powerwatch-web
powerwatch-hub
```

### Lancer les binaires

CLI :

```bash
./target/release/powerwatch
./target/release/powerwatch --json
```

TUI :

```bash
./target/release/powerwatch-tui
```

WebUI locale avec historique :

```bash
./target/release/powerwatch-web   --host 127.0.0.1   --port 3000   --log   --history-interval 60
```

Pour un serveur/headless, ajoutez `--nas-mode`. L'authentification facultative s'active avec `--auth` ou `POWERWATCH_AUTH_ENABLED=true` ; sans elle, limitez une écoute LAN à un réseau privé de confiance.

Hub avec ses fichiers de configuration et d'historique dans le répertoire courant :

```bash
./target/release/powerwatch-hub   --host 127.0.0.1   --port 3065   --config ./powerwatch-hub.json   --database ./powerwatch-hub.db
```

### Accès au matériel

En natif, PowerWatch lit directement les interfaces matérielles exposées par Linux. Il n'y a donc pas de montage Docker à configurer.

- **NVIDIA** : NVIDIA Container Toolkit n'est pas nécessaire en natif. Le pilote NVIDIA/NVML doit simplement fonctionner sur l'hôte ; `nvidia-smi` permet de le vérifier.
- **AMD / Intel** : les compteurs disponibles dans `/sys` sont lus directement.
- **RAPL MSR** : si le fallback `/dev/cpu/0/msr` est nécessaire, le processus doit disposer des permissions/capabilities exigées par le noyau pour ouvrir le device, notamment `CAP_SYS_RAWIO` selon la configuration de l'hôte.

> **Support :** la compilation native est fournie comme possibilité avancée. Les dépendances, permissions, chemins matériels et comportements peuvent varier selon la distribution et le noyau. Le déploiement Docker reste la référence documentée et testée par la CI PowerWatch.

## GPU

### AMD

Aucune configuration Docker supplémentaire n'est nécessaire. PowerWatch lit les compteurs Linux `amdgpu` exposés dans `/sys`.

### Intel

PowerWatch utilise en priorité les compteurs `i915` / `xe` disponibles dans `hwmon`.

Lorsqu'ils ne fournissent pas de télémétrie de puissance mais que RAPL expose un sous-domaine `uncore`, celui-ci peut être utilisé comme fallback `gpu:intel:0`.

### NVIDIA

Le support NVIDIA repose sur **NVML**. Le pilote NVIDIA doit fonctionner sur l'hôte et **NVIDIA Container Toolkit doit être installé puis configuré pour Docker**.

Commencez par vérifier le pilote sur l'hôte :

```bash
nvidia-smi
```

Puis vérifiez/installez NVIDIA Container Toolkit et configurez le runtime Docker :

```bash
sudo nvidia-ctk runtime configure --runtime=docker
sudo systemctl restart docker
```

Avant de lancer PowerWatch, validez impérativement l'accès GPU depuis Docker :

```bash
docker run --rm --runtime=nvidia --gpus all ubuntu nvidia-smi
```

Si ce test affiche le GPU, Docker/NVIDIA est correctement configuré. Vous pouvez alors décommenter dans `compose.yaml` les variables `NVIDIA_VISIBLE_DEVICES` / `NVIDIA_DRIVER_CAPABILITIES` ainsi que le bloc `deploy.resources.reservations.devices`, puis recréer PowerWatch :

```bash
docker compose up -d --force-recreate
```

Si Docker renvoie :

```text
could not select device driver "nvidia" with capabilities: [[gpu]]
```

le problème se situe **avant PowerWatch** : Docker ne dispose pas encore d'un runtime NVIDIA utilisable. Un montage manuel de `nvidia-smi` dans le conteneur ne corrige pas ce problème.

Sur Ubuntu/Debian, si `nvidia-container-toolkit` n'est pas encore disponible dans vos dépôts configurés, suivez l'installation officielle NVIDIA avant d'exécuter `nvidia-ctk` : [NVIDIA Container Toolkit — Install Guide](https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html).

`NVIDIA_DRIVER_CAPABILITIES: utility` suffit à PowerWatch pour accéder à NVML ; aucune pile graphique n'est nécessaire. Plusieurs GPU et les machines mixtes AMD/Intel/NVIDIA sont supportés lorsque les compteurs correspondants sont lisibles.

PowerWatch lit d'abord la puissance directe via NVML. Si cette lecture échoue mais que le pilote fournit des échantillons de puissance, il utilise le plus récent pour mesurer le GPU et l'inclure dans le total. Pour vérifier cette situation, comparez `nvidia-smi --query-gpu=power.draw --format=csv` et la section « Power Samples » de `nvidia-smi -q -d POWER`. Si les deux sources sont indisponibles, PowerWatch signale une erreur de lecture.

## Disques, partitions et stockage

PowerWatch crée un capteur électrique uniquement pour chaque **disque physique**. Les couches comme mdraid, LVM et dm-crypt ne sont pas ajoutées comme faux disques et ne sont donc pas comptées une seconde fois.

Quand Linux permet de reconstruire la relation, seules les chaînes qui aboutissent à un **montage réellement utile** sont affichées, par exemple :

```text
disk (sda) — sda1 → md0 [RAID1] → vg-data/lv-media [LVM] → /mnt/data [ext4]

disk (nvme0n1) — nvme0n1p2 → cryptroot [dm-crypt] → / [ext4]
```

Les partitions non montées sont volontairement masquées. Si un disque n'a aucune partition montée, PowerWatch conserve simplement le disque, sa capacité et sa consommation estimée sans ajouter de liste `[unmounted]`. Si certaines partitions sont montées et d'autres non, seules les partitions montées apparaissent.

La capacité de chaque disque physique est lue depuis sysfs (`/sys/class/block/<device>/size`) et affichée avec son libellé enrichi. Les identifiants internes restent stables (`disk:sda`, `disk:nvme0n1`, etc.) afin de ne pas casser l'historique ni les alertes.

### Estimation de la consommation des disques

Les valeurs disque restent des **estimations**, pas des mesures électriques directes. Pour les profils génériques, PowerWatch utilise le temps pendant lequel le périphérique est occupé dans `/proc/diskstats` entre deux échantillons et interpole entre une valeur de repos et une valeur active :

| Type détecté | Repos | Actif | Détection |
|---|---:|---:|---|
| HDD rotatif | 4 W | 8 W | `queue/rotational = 1` |
| SSD non amovible | 0,5 W | 3 W | non rotatif et non amovible |
| Flash basse consommation (clé USB amovible, eMMC/SD) | 0,2 W | 1,5 W | `removable = 1` ou périphérique `mmcblk*` |
| NVMe, fallback générique | 1 W | 6 W | périphérique `nvme*` |

Par exemple, un SSD occupé environ 50 % de l'intervalle est estimé à mi-chemin entre 0,5 W et 3 W au lieu de basculer immédiatement à la valeur active maximale dès la moindre I/O. Le ratio est borné entre 0 et 100 %.

Pour les NVMe, PowerWatch essaie d'abord d'utiliser `nvme-cli` et le power state courant exposé par sysfs. Lorsque cette information est disponible, la valeur maximale annoncée par le constructeur pour le power state courant est utilisée. Si elle ne l'est pas, PowerWatch revient au profil générique NVMe ci-dessus.

La détection des médias amovibles dépend des informations exposées par le noyau : un boîtier USB peut donc être classé comme HDD, SSD ou flash selon `rotational` et `removable`. La confiance reste dans tous les cas **Estimée**.

## WebUI

<p align="center">
  <img src="docs/images/instancemini.png" alt="WebUI PowerWatch : capteurs, consommation totale et historique">
</p>
<p align="center">
  <img src="docs/images/instancefull.png" alt="PowerWatch — vue large, thème sombre">
</p>


La page principale affiche :

- la consommation totale ;
- chaque capteur détecté ;
- la distinction **mesurée / estimée** ;
- l’historique graphique en haut de page, à côté du total, avec les durées Direct, 1 h, 6 h, 24 h, 7 j, 30 j, 180 j et 400 j ainsi qu’une période personnalisée ;
- moyenne, minimum, maximum et énergie ;
- des périodes prédéfinies et personnalisables ;
- les libellés enrichis des disques avec leur capacité ;
- le modèle CPU avec un lien vers CPU Benchmark / PassMark ;
- un nom d'instance modifiable, réutilisé comme suggestion lors de l'ajout dans PowerWatch Hub ;
- le choix de langue FR / EN ;
- les boutons **Alertes**, **MQTT** et **Tarifs** en haut à droite : les réglages Home Assistant et tarification ne prennent plus de place sur l’index ;
- le sélecteur de thème **Clair / Sombre / Auto**, avec palettes de jour et de nuit inspirées de Dockge-Enhanced. Les réglages restent dans le navigateur, séparément pour chaque adresse de WebUI et de Hub.

L'image Docker démarre automatiquement PowerWatch avec l'historique activé et le mode NAS/headless.

### Tarification électrique France — bêta (optionnelle)

<p align="center">
  <img src="docs/images/tarifs.png" alt="PowerWatch — Tarifs / Tariffs">
</p>

**Bêta : les prix peuvent être incomplets, périmés ou erronés. Les coûts sont indicatifs, non mesurés au compteur et ne remplacent pas une facture.** Désactivée par défaut. Depuis le menu **Tarifs** en haut à droite de chaque instance ou du Hub, l’interrupteur « Activer la tarification et afficher les coûts » enregistre immédiatement le choix. Une fois désactivée, les montants disparaissent des compteurs, mais les contrats restent enregistrés. La base SQLite conserve les configurations et les corrections manuelles ; rien n'est enregistré exclusivement dans le navigateur.

- Contrats et fournisseurs successifs avec dates de début/fin, sites et affectation des machines côté Hub. Un seul contrat à la fois par site évite les doubles tarifs ; plusieurs sites distincts peuvent avoir des fournisseurs différents.
- Versions de tarifs datées par contrat : Base, heures pleines/creuses personnalisables, Tempo avec ses six prix, offre personnalisée. Chaque prix TTC, abonnement annuel et date restent éditables ; toute correction manuelle prime sur la source automatique.
- Synchronisation des **tarifs réglementés EDF Tarif Bleu** via fichiers CSV officiels de la CRE (Base, HP/HC, Tempo), selon la puissance souscrite ; synchronisation des **couleurs Tempo confirmées** via l'API tierce calendrier-tempo.fr (CC BY 4.0 ; ce n'est pas une API officielle EDF/RTE). D'abord manuellement par bouton, puis chaque jour côté serveur lorsque l'option est activée. L'indisponibilité d'une source n'efface jamais les tarifs existants.
- **Catalogue automatique bêta (France)** : sélection fournisseur → offre → option Base / HP-HC → puissance souscrite ; récupération en direct des grilles publiques TTC (prix du kWh et abonnement) publiées par Kelwatt, avec source et date de consultation. Mise à jour côté serveur chaque jour pour les offres importées, sans clé API ni saisie des prix. Si la structure du site change, l'import échoue explicitement sans inventer de prix ; les anciennes grilles restent enregistrées.
- **Parcours simple** : « Mon contrat et mes prix » est le formulaire principal. Le bouton « Modifier ce contrat et ses prix » recharge les données enregistrées pour corriger une valeur sans tout ressaisir. La recherche d'une nouvelle offre dans le catalogue est repliée séparément. Pour HP/HC, saisissez les horaires réels du compteur (facture ou Enedis). « Tarif Bleu » correspond uniquement à un contrat EDF enregistré. Contrats et grilles, Tempo, Estimation de coût et JSON restent dans « Réglages avancés ».
- **Prix historiques** : si votre fournisseur confirme que **les prix affichés** s'appliquaient déjà à une date antérieure, renseignez et confirmez cette date. Sinon, le prix enregistré commence aujourd'hui. Les relevés antérieurs restent sans prix documenté ; l'estimation facultative ci-dessous permet d'afficher un ordre de grandeur distinct. Une grille actuelle ne prouve pas un prix historique.
- **Offre ancienne ou absente du catalogue (par exemple « Online » depuis 2019)** : saisissez fournisseur, offre, option, puissance, prix TTC connus sur facture ou espace client, abonnement **mensuel TTC** et horaires HC. Si la date d'effet des prix est inconnue, laissez-la vide : les prix sont enregistrés à partir d'aujourd'hui, après confirmation, sans les attribuer à 2019. « Enregistrer le contrat sans prix » reste possible. Le bouton « Modifier ce contrat et ses prix » préremplit le formulaire et permet de corriger la version tarifaire courante sans supprimer l'historique. Un ancien « Tarif Bleu » sans prix peut être remplacé après confirmation. Aucun document PDF, nom, adresse ou identifiant de client/compteur n'est importé dans PowerWatch.
- **Estimation facultative de l'historique** : cochez « Estimer mes anciens relevés avec le premier prix connu » pour afficher séparément un montant approximatif sur les relevés antérieurs à cette première grille, à condition qu'un contrat et les horaires HC nécessaires existent. Les prix historiques enregistrés ne changent pas ; les périodes sans contrat ou sans catégorie tarifaire restent indisponibles. Le montant est libellé « estimation historique au prix connu » et ne représente pas une facture ancienne.
- **Limites importantes** : les offres à plusieurs plages spéciales (ex. Heures Eco+ ou Charge'Heures) ne sont pas interprétées comme de simples contrats HP/HC. Les grilles publiques courantes ne garantissent pas les prix d'un ancien contrat fixe ou une promotion personnelle ; les tarifs historiques ne sont pas reconstitués. Les heures creuses **attribuées au compteur Linky** ne sont pas déductibles d'un catalogue de fournisseurs : elles doivent être vérifiées auprès d'Enedis / du fournisseur. Sans horaires validés, aucun calcul HP/HC fictif. Une saisie et des corrections manuelles restent possibles.
- Date et heure en `Europe/Paris`, avec changement d'heure. La journée Tempo va de 06 h à 06 h, HP de 06 h à 22 h pour Tempo. La grille HP/HC non Tempo est configurable pour chaque contrat.
- **Agrégats énergétiques de 15 minutes conservés sans limite de durée programmée** pour les nouvelles archives. Les anciens agrégats horaires restent lisibles, avec un avertissement de précision pour leurs coûts. L'estimation de l'historique ne crée aucune grille tarifaire datée fictive.
- Abonnements facultatifs, calculés une fois par site et proratisés au jour ; aucun abonnement doublement compté entre plusieurs machines du Hub. Les périodes sans contrat ou couleur Tempo restent en kWh non chiffrés ; les périodes sans prix antérieur peuvent être projetées seulement si l'option d'estimation est activée.
- Accès WebUI : `/api/tariffs-fr` (GET/PUT), `/api/tariffs-fr/sync` (POST), `/api/tariffs-fr/cost` (GET) et `/api/tariffs-fr/overview` (GET, compteurs). Accès Hub : `/api/hub/tariffs-fr` et équivalents `/sync`, `/cost`, `/overview` (`nodes` facultatif pour les instances sélectionnées). Les mutations sont protégées par l'authentification de la WebUI ou l'authentification administrateur du Hub si activée.
- **Copie volontaire entre machines** : dans le menu **Tarifs** en haut du Hub, ouvrez « ⇄ Tarifs », choisissez l'instance source et cochez uniquement les machines ayant le même contrat et les mêmes horaires HC. Cochez « Copier aussi vers le Hub » pour calculer leurs coûts avec cette grille. La copie remplace la configuration tarifaire des machines cochées ; les autres restent intactes. Si l'authentification PowerWatch est activée, créez dans « Sécurité » de chaque destinataire un jeton « Lire et modifier les tarifs électriques » et renseignez-le dans la fiche de cette machine dans le Hub. Le jeton habituel en lecture seule reste distinct. La copie est déclenchée par le bouton et affiche les réussites et échecs par machine.

Sources : https://www.cre.fr/documents/open-data/historique-des-tarifs-reglementes-de-vente-delectricite-pour-les-consommateurs-residentiels.html et https://www.calendrier-tempo.fr/api-tempo. Aucun tarif n'est codé en dur.

## Consommation énergétique cumulée (kWh)

> **Coût dans les compteurs (instance et Hub)** : lorsque la tarification française est activée, les quatre compteurs 24 h, 7 j, 30 j et historique affichent automatiquement le montant estimé en euros sous les kWh. Les périodes personnalisées utilisent le même moteur tarifaire. Dans le Hub, le coût suit le filtre des instances et l’abonnement éventuel est compté une seule fois par site électrique. Si une tranche manque de contrat, de prix, d'horaire HC ou de couleur Tempo, aucun total en euros incomplet n'est présenté comme un coût complet ; la raison est affichée.


PowerWatch et PowerWatch Hub affichent **24 h, 7 j, 30 j et cumul depuis les premiers relevés**, ainsi qu'une période personnalisée (dates et heures). Le Hub présente aussi les kWh **par machine**.

Les kWh sont intégrés entre relevés successifs, uniquement si l'écart est compatible avec la fréquence de collecte ; les arrêts, capteurs indisponibles et longues coupures ne sont **pas interpolés**. La durée réellement couverte est indiquée dans l'interface (pour le Hub, en heures-machine). Le total peut donc être **inférieur à la consommation réelle**. La puissance de composants tels que RAM/disques étant estimée et les pertes d'alimentation non mesurées, **ces kWh ne remplacent pas un wattmètre à la prise**.

Les cumuls sont persistés en SQLite, indépendamment de la rétention de l'historique en watts. Les intervalles précis sont conservés 45 jours, puis regroupés en quarts d’heure sans perdre le cumul ; les anciennes archives horaires restent lisibles. Lors de la première migration, seuls les relevés bruts historiques réellement disponibles et suffisamment proches sont convertis : **aucune consommation ne peut être reconstituée durant les périodes non observées**.

- API locale : `GET /api/energy?from=<ISO8601>&to=<ISO8601>` (paramètres facultatifs).
- API Hub : `GET /api/hub/energy?from=<ISO8601>&to=<ISO8601>` (périodes globales et par machine).
- La WebUI locale conserve sa carte « Énergie » liée à l'historique graphique ; les nouveaux compteurs restent indépendants du graphique.

## Historique

Les données sont stockées dans SQLite dans le volume persistant `./data`.

Rétention actuelle :

- mesures brutes : **30 jours** ;
- agrégats 15 minutes : **de 30 jours à 1 an** ;
- agrégats 1 heure : **au-delà d'un an**.

Les longues périodes sont agrégées côté serveur afin d'éviter d'envoyer inutilement des milliers de points à la WebUI.

## Alertes et notifications

<p align="center">
  <img src="docs/images/alertes.png" alt="PowerWatch Alerts">
</p>

La page `/alerts` permet de créer des règles persistantes sur :

- le total ;
- le CPU ;
- l'ensemble des GPU ;
- un GPU précis (`gpu:nvidia:0`, `gpu:amd:0`, `gpu:intel:0`, etc.) ;
- la RAM ;
- un disque précis.

Chaque règle peut définir :

- un seuil en watts ;
- une durée minimale de dépassement ;
- activation / désactivation ;
- notification de retour à la normale.

Notifications disponibles :

- **Discord** via webhook entrant ;
- **Apprise API**.

Les paramètres d'alertes sont conservés dans le volume persistant Docker avec l'historique.

## Authentification intégrée

PowerWatch peut protéger sa WebUI et ses API avec une authentification native, sans popup HTTP ni service externe. Elle est **désactivée par défaut** afin de préserver les installations existantes.

Pour la première activation, ajoutez dans `.env` :

```dotenv
POWERWATCH_AUTH_ENABLED=true
POWERWATCH_AUTH_SETUP_TOKEN=un-secret-aleatoire-long-et-unique
```

Le jeton d'initialisation doit contenir au moins 16 caractères. Redémarrez PowerWatch, ouvrez la WebUI puis saisissez ce jeton dans l'écran de création du compte. Il empêche un visiteur ayant découvert l'URL avant l'administrateur de s'approprier l'unique compte.

Après la création du compte, supprimez `POWERWATCH_AUTH_SETUP_TOKEN` de `.env` et recréez le conteneur. Le compte, les sessions et les jetons d'intégration restent dans `./data/auth.json`, avec l'historique et les autres données persistantes. `POWERWATCH_AUTH_ENABLED=true` doit rester défini.

La page **Sécurité** permet de :

- changer le mot de passe ;
- révoquer toutes les sessions ;
- créer et révoquer des jetons API Bearer en lecture seule ;
- se déconnecter.

Les mots de passe sont hachés avec **Argon2id**. Les sessions sont conservées côté serveur ; le navigateur reçoit uniquement un cookie `HttpOnly`, `SameSite=Strict`, automatiquement marqué `Secure` lorsque le reverse proxy transmet `X-Forwarded-Proto: https`. Les actions par session exigent également un jeton CSRF.

### API pour PowerWatch Hub et Dockge-Enhanced

Créez un jeton depuis **Sécurité**, copiez-le immédiatement — sa valeur complète ne sera plus affichée — puis envoyez-le dans l'en-tête HTTP :

```http
Authorization: Bearer pw_...
```

Le jeton donne uniquement accès en lecture à :

- `GET /api/snapshot` ;
- `GET /api/history` ;
- `GET /api/history/range` ;
- `GET /api/instance`.

Les endpoints d'authentification sont :

- `GET /api/auth/status` — état public, sans secret ;
- `POST /api/auth/setup` — création initiale avec `username`, `password` et `setup_token` ;
- `POST /api/auth/login` et `POST /api/auth/logout` ;
- `GET /api/auth/settings` ;
- `POST /api/auth/password` ;
- `POST /api/auth/sessions/revoke` ;
- `POST /api/auth/tokens` et `DELETE /api/auth/tokens/:id`.

`GET /api/health` reste public. Les autres API de données ou d'action refusent les appels non authentifiés lorsque l'option est active. PowerWatch Hub accepte le jeton facultatif dans le formulaire de chaque instance et ne l'expose jamais dans ses réponses API.

Les opérations d'administration du Hub peuvent elles aussi être protégées, de manière facultative :

```dotenv
POWERWATCH_HUB_ADMIN_TOKEN=un-secret-aleatoire-d-au-moins-24-caracteres
```

Lorsque cette variable est définie, l'ajout, la liste, la modification, la suppression et le test des instances exigent `Authorization: Bearer <jeton administrateur>`. Le dashboard permet de saisir ce secret pour l'onglet courant ; il n'est ni renvoyé par l'API ni ajouté aux URLs. `GET /api/hub/auth/status` indique uniquement si cette protection est active. Sans variable, le comportement historique du Hub reste inchangé.

Un jeton d'instance déjà enregistré reste associé à son URL actuelle. Si l'URL est modifiée sans fournir explicitement un nouveau jeton dans la même requête, le secret enregistré est supprimé au lieu d'être transmis à la nouvelle destination. Les modifications qui conservent la même URL préservent le jeton existant.

Pour un reverse proxy HTTPS, terminez TLS sur le proxy, transmettez `X-Forwarded-Proto: https` et remplacez toute valeur fournie par le client au lieu de la concaténer. PowerWatch n'embarque pas lui-même de certificat TLS.

## CLI et TUI dans Docker

La WebUI est le service principal, mais l'image contient également la CLI et la TUI.

### Snapshot

```bash
docker exec powerwatch powerwatch
```

### JSON

```bash
docker exec powerwatch powerwatch --json
```

### Historique CLI

```bash
docker exec powerwatch powerwatch history --since 1h
```

### TUI

<p align="center">
  <img src="docs/images/Image_TUI.png" alt="PowerWatch TUI">
</p>

```bash
docker exec -it powerwatch powerwatch-tui
```

Lancer la CLI ou la TUI avec `docker exec` n'interrompt pas la WebUI.

## Vérifier les capteurs détectés

```bash
docker exec powerwatch powerwatch --json
```

Exemples d'identifiants GPU :

```text
gpu:nvidia:0
gpu:nvidia:1
gpu:amd:0
gpu:intel:0
```

Pour vérifier RAPL directement dans le conteneur :

```bash
docker exec powerwatch sh -c \
  'find /host-sys-virtual/powercap/intel-rapl -name energy_uj -o -name name 2>/dev/null'
```



## PowerWatch Hub — plusieurs machines, un seul dashboard

PowerWatch peut également fonctionner en mode **Hub**. Chaque machine conserve son instance PowerWatch locale et sa collecte matérielle ; le Hub interroge simplement leurs API HTTP et les regroupe dans une seule WebUI.

<p align="center">
  <img src="docs/images/hubminiEN.png" alt="PowerWatch Hub : vue détaillée des instances fédérées">
</p>

<p align="center">
  <img src="docs/images/hubfull.png" alt="PowerWatch Hub : vue large du total et de l'historique fédéré">
</p>

Le Hub fournit :

- un **total global** de l'infrastructure, accompagné de la mention **« Mesures + estimations »** ;
- une carte repliable par machine avec CPU, GPU, RAM et disques ;
- les états **online / stale / offline / disabled** ;
- l'ajout, le test, l'activation et la suppression d'instances depuis le bouton **Machines / instances** en haut à droite ;
- la possibilité d'exclure une machine du total global ;
- le filtrage des cumuls énergétiques par une, plusieurs ou toutes les instances, indépendamment du total global ;
- un historique SQLite fédéré, enregistré par défaut toutes les 60 secondes ;
- la récupération automatique de l’historique agrégé déjà présent sur les nœuds (jusqu’à 400 jours) ;
- un graphe fédéré avec **total global + une courbe colorée par instance** ;
- des cartes compactes avec résumé CPU / GPU / RAM / disques et couleurs mesurée / estimée ;
- une interface FR / EN alignée visuellement sur la WebUI PowerWatch.

Le total global additionne les valeurs remontées par les instances, **qu'elles soient mesurées ou estimées**. Il ne correspond pas à une mesure de consommation à la prise et n'inclut pas la puissance nominale des alimentations.

Le Hub ne nécessite **aucun accès à `/sys`, aucun `pid: host` et aucun accès privilégié**. Il doit seulement pouvoir joindre les URLs privées des instances PowerWatch.

### Sélection des instances pour les cumuls

Dans le panneau **Consommation énergétique cumulée** du Hub, le filtre **Instances pour les cumuls** choisit les machines prises en compte :

- **Total habituel** (par défaut) : seules les instances activées et marquées **« compter dans le total »** sont additionnées.
- **Toutes** : toutes les instances configurées, y compris celles exclues du total habituel ou désactivées, dans la limite de leur historique disponible.
- **Sélection personnalisée** : cochez les instances souhaitées pour additionner une seule machine ou plusieurs. Si aucune n'est cochée, le panneau demande d'en sélectionner au moins une.

La sélection s'applique aux **24 h, 7 j, 30 j, cumul historique, période personnalisée** et à la **couverture en heures-machine**. Elle est mémorisée dans le navigateur (`localStorage`) et ne modifie **ni les données SQLite, ni le total instantané en watts, ni le graphique fédéré, ni les cumuls des cartes individuelles**.

### Démarrage du Hub

```bash
mkdir -p hub-data
docker compose -f compose.hub.yaml pull
docker compose -f compose.hub.yaml up -d
```

Par défaut, le dashboard Hub écoute sur :

```text
http://127.0.0.1:3065
```

Pour l'exposer sur le LAN privé :

```env
POWERWATCH_HUB_BIND_IP=192.168.0.50
POWERWATCH_HUB_PORT=3065
```

Les machines s'ajoutent ensuite depuis la WebUI avec leur URL PowerWatch, par exemple :

```text
Garuda       http://192.168.0.53:3064
LincStation  http://192.168.0.196:3064
DockerLab    http://192.168.0.2:3064
```

La configuration est stockée dans `hub-data/powerwatch-hub.json` et l'historique fédéré dans `hub-data/powerwatch-hub.db`.

> Au démarrage du Hub et lors de l’ajout/réactivation d’une instance, PowerWatch Hub récupère via `/api/history/range` l’historique agrégé disponible sur le nœud (jusqu’à 400 jours). L’import est idempotent : les mêmes points ne sont pas dupliqués dans SQLite.

## Home Assistant — découverte MQTT (facultative)

<p align="center">
  <img src="docs/images/mqttoption.png" alt="PowerWatch — MQTT settings">
</p>

PowerWatch et PowerWatch Hub peuvent désormais **publier directement leurs capteurs dans Home Assistant** via **MQTT Discovery** : aucun YAML ni composant HACS n'est nécessaire. Dans **chaque WebUI**, ouvrez **MQTT** en haut à droite, puis activez le commutateur. L'intégration est **désactivée par défaut** et ne lance aucune connexion au broker tant qu'elle reste désactivée.

1. Dans Home Assistant, installez/configurez l'intégration **MQTT** ainsi qu'un broker (par exemple l'add-on Mosquitto). Le broker doit être joignable **depuis le conteneur PowerWatch/Hub** : utilisez son IP ou son nom DNS réseau (pas `localhost`, sauf si le broker est dans le même conteneur).
2. Ouvrez le bouton **MQTT** en haut à droite de PowerWatch ou du Hub, puis renseignez **adresse, port** (1883 en clair, généralement 8883 en TLS), identifiant et mot de passe si nécessaires. Cochez **TLS** si le broker a un certificat valide reconnu par le système ; les certificats auto-signés non approuvés ne sont pas acceptés par défaut.
3. Choisissez la fréquence (10–600 s), l'export des composants, des cumuls kWh et des **coûts estimés en euros** (uniquement avec la tarification France active), puis, dans le Hub, la portée : **Hub + machines**, **Hub uniquement** (par défaut, pour éviter les doublons) ou **machines uniquement**. Enregistrez et cliquez sur **Tester la connexion** : ce bouton teste la négociation MQTT et les identifiants, pas seulement l'ouverture du port.
4. Les équipements « PowerWatch » apparaissent automatiquement sous **Paramètres → Appareils et services → MQTT** dans Home Assistant, avec les capteurs **W** (puissance totale et composants), **kWh** historiques par machine et, si le tarif est complet, **€** estimés, dès que PowerWatch dispose de données.

### Captures Home Assistant — test MQTT réel

<!-- POWERWATCH_HOME_ASSISTANT_SCREENSHOTS_START -->
<p align="center">
  <img src="docs/images/PowerWatch-HomeAssistant-MQTT-Devices.png" alt="Home Assistant : découverte MQTT de trois appareils PowerWatch et de 29 entités" width="920">
</p>
<p align="center"><em>Découverte automatique dans Home Assistant : trois appareils PowerWatch et 29 entités MQTT.</em></p>

<p align="center">
  <img src="docs/images/PowerWatch-HomeAssistant-MQTT-Sensors.png" alt="Home Assistant : capteurs PowerWatch de puissance en W, d'énergie cumulée en kWh et de coûts estimés en EUR" width="1080">
</p>
<p align="center"><em>Exemple d'entités : puissance des composants, puissance totale, 2,403 kWh cumulés et coûts estimés (0,53 € d'énergie + 0,06 € d'abonnement = 0,59 €).</em></p>

**Captures issues d'un laboratoire Docker isolé (PowerWatch, Hub, Mosquitto et Home Assistant).** Les consommations historiques et les prix utilisés pour tester les capteurs en euros sont **fictifs** (2,4 kWh injectés, 0,22 €/kWh et abonnement de 180 €/an). Ces montants ne représentent ni un tarif commercial réel ni une facture.
<!-- POWERWATCH_HOME_ASSISTANT_SCREENSHOTS_END -->

### Comportement et limites

- **Instance autonome** : un appareil Home Assistant, puissance totale, composants optionnels, compteur énergétique historique lorsque la base SQLite contient des échantillons. Le cumul requiert que la journalisation `--log` soit activée (déjà le cas dans le Compose fourni).
- **Hub** : un appareil Hub pour la puissance globale instantanée **et son propre compteur kWh cumulatif persistant**, et/ou un appareil par machine fédérée (W, composants, kWh issus de l'historique du Hub). Les machines hors ligne passent en **indisponible**, jamais artificiellement à `0 W`. Une machine doit avoir des données valides pour publier ses capteurs.
- **Cumul kWh global Home Assistant du Hub** : un compteur **distinct du cumul historique filtrable actuel**, enregistré dans `powerwatch-hub.db`. Il est **initialisé une seule fois à partir des kWh historiques déjà enregistrés pour les machines incluses lors de la mise à jour** (ou à 0 si aucun historique n’existe), puis ajoute les kWh de chaque nouvel intervalle réellement observé (machine active, disponible et incluse dans le total), sans jamais soustraire les kWh passés. Exclure, supprimer ou éteindre une machine suspend ses contributions sans faire baisser le compteur ; son retour ou l'ajout d'une nouvelle machine ne comptabilise que les nouveaux intervalles. Un redémarrage du Hub conserve sa valeur ; les interruptions de plus de 180 secondes ne sont pas interpolées. **Ce compteur ne rajoute pas rétroactivement d'historiques importés après son initialisation** et continue d'être comptabilisé même lorsque MQTT est désactivé.
- **Le cumul historique du Hub reste inchangé** : ses filtres « total habituel / toutes / sélection personnalisée » peuvent recalculer une somme différente selon le périmètre ; ceci ne modifie jamais le compteur global MQTT. Pour éviter le double comptage dans le tableau Énergie Home Assistant, choisissez **soit** le compteur global Hub, **soit** les compteurs des machines, mais pas les deux ensemble.
- **Export des coûts en euros (optionnel)** : reprend directement le moteur de tarification France de PowerWatch, sans recalcul indépendant. Pour une instance autonome : coût de consommation estimé, abonnement facultatif et total (si abonnement activé). Pour le Hub : coûts globaux des **machines actuellement incluses** avec abonnement compté **une seule fois par site**, et, si les machines individuelles sont exportées, uniquement leur coût variable (sans répéter l'abonnement). Les capteurs sont des `sensor` MQTT avec `device_class: monetary`, `state_class: total` et l’unité `EUR` : **les euros ne sont pas un compteur `total_increasing`**, car tarifs, corrections historiques et périmètre du Hub peuvent modifier l'estimation. N’additionnez pas `coût énergie`, `abonnement` et `coût total` : le total comprend déjà les deux premiers. Le montant global en € peut donc différer du périmètre du compteur kWh global HA persistant du Hub si la sélection a changé.
- **Tarifs incomplets** : sans tarification active, sans historique facturable ou en présence de kWh non chiffrés/projetés, aucun montant « 0 € » trompeur n'est publié. Si seule la part fixe de l’abonnement est absente ou projetée, **le coût variable de l’énergie reste exporté**, mais les capteurs « abonnement » et « coût total » ne le sont pas. Les anciens capteurs € MQTT sont nettoyés lorsque leurs données deviennent invalides ou que l'option est désactivée. Aucune grille de prix n’est inventée.
- Les watts peuvent être **mesurés, estimés ou partiels** selon le matériel (PMIC, RAPL, etc.) ; les données **ne représentent pas nécessairement la consommation à la prise**. Les montants € sont des **estimations tarifaires**, pas des montants de factures réelles ni un relevé certifié à la prise.
- **Disponibilité** : message LWT du client MQTT en cas de coupure, disponibilité par machine, et expiration automatique des données de puissance en l'absence de nouveaux relevés. La découverte ainsi que les compteurs kWh et les coûts € sont publiés en **retained**, les relevés de puissance ne le sont pas.
- **Sécurité** : les mots de passe ne sont pas renvoyés par l'API ; champ vide = conservation du secret, option distincte pour l'effacer. Les réglages sont persistés en JSON avec permissions restreintes (`0600` sous Linux), dans le volume de données. Les routes de configuration respectent l'authentification WebUI et le jeton administrateur facultatif du Hub. Utilisez un compte MQTT dédié avec permissions minimales sur les topics choisis.
- **Double export** : si le Hub et une instance publient la même machine, Home Assistant affiche deux équipements distincts pour cette machine ; choisissez idéalement le Hub **ou** l'instance comme source afin d'éviter les doubles comptages dans vos tableaux.
- Les identifiants Discovery restent stables grâce à un identifiant enregistré localement. L’inventaire des topics annoncés est aussi enregistré sur disque : les capteurs supprimés, les changements de sélection et les anciens préfixes sont nettoyés automatiquement **lorsque le même broker est accessible**. Une machine simplement hors ligne conserve ses capteurs, signalés indisponibles. Si le broker est inaccessible lors de la désactivation, ou si l’adresse du broker change, des anciens topics retained peuvent nécessiter un nettoyage manuel sur l’ancien broker.

**Fichiers persistants :** compteur global MQTT du Hub dans `hub-data/powerwatch-hub.db` (sauvegarder ce fichier avec la base historique) ; instance `~/.local/share/powerwatch/home-assistant-mqtt.json` (dans Docker : `/data/.local/share/powerwatch/home-assistant-mqtt.json`) ; Hub : `hub-data/powerwatch-hub.mqtt.json` pour le Compose fourni. Les fichiers `home-assistant-mqtt.inventory.json` (instance) et `powerwatch-hub.mqtt.inventory.json` (Hub) conservent l’inventaire de découverte dans les mêmes volumes. L'intégration est désactivée tant que vous ne l'activez pas dans la WebUI. L'export fonctionne vers n'importe quel broker MQTT compatible, sans dépendance de Home Assistant pour les mesures PowerWatch elles-mêmes.

## Sécurité

PowerWatch a besoin d'accéder à plusieurs informations matérielles de l'hôte pour mesurer les composants.

Le Compose fourni utilise notamment :

- `/sys` en lecture seule ;
- `/sys/devices/virtual` en lecture seule pour RAPL ;
- `pid: host` pour certaines informations hôte ;
- un système de fichiers conteneur en lecture seule ;
- `no-new-privileges:true`.

La WebUI propose une authentification intégrée facultative, mais pas de terminaison HTTPS. Pour une exposition via reverse proxy, activez l'authentification et utilisez HTTPS ; sans authentification, limitez l'accès à un LAN privé de confiance.


## Attribution

Projet original : [lnpotter/PowerWatch](https://github.com/lnpotter/PowerWatch)

Merci à son auteur pour la base du projet.

## Licence

Voir [`LICENSE`](LICENSE).
