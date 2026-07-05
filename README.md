# RustVpsMon

Agent de monitoring "tout-en-un" pour VPS, compilé en un seul binaire. Surveille le CPU/RAM/disque de l'hôte, découvre et monitore les conteneurs Docker, garde un historique en SQLite, et expose un tableau de bord web temps réel (HTMX + SSE, sans JS lourd).

Spécification fonctionnelle complète : [`specification.md`](specification.md).

## Fonctionnalités

- Collecte CPU / RAM / disque de l'hôte toutes les N secondes (`sysinfo`).
- Découverte et monitoring des conteneurs Docker via le socket Unix (`bollard`).
- Dashboard web temps réel sans rechargement de page (SSE + HTMX).
- Historique des métriques en SQLite, avec purge automatique au-delà de la rétention configurée.
- Alertes avec anti-spam (une notification par incident, pas une par cycle de collecte), acquittement depuis l'interface, notification de retour à la normale.
- Notifications par e-mail (SMTP via `lettre`) et/ou webhook JSON (Discord/Slack/Gotify... via `reqwest`).
- Binaire unique : assets et templates sont embarqués à la compilation (`rust-embed`, `askama`).
- Empreinte mémoire visée : ~20-30 Mo de RAM en vitesse de croisière (mesuré en pratique ~11 Mo).

## Démarrage rapide

### En local

```bash
cp .env.example .env   # ajuster les seuils/notifications si besoin
cargo run --release
```

Le dashboard est servi sur `http://localhost:3000` (ou l'adresse définie par `RUSTMON_BIND_ADDR`).

### En Docker

```bash
docker compose up -d --build
```

Le `docker-compose.yml` fourni monte le socket Docker (découverte des conteneurs), le disque racine de l'hôte en lecture seule sur `/host` (métriques disque réelles du VPS, via `RUSTMON_DISK_PATH=/host`), et un volume `./data` pour persister la base SQLite. Un fichier `.env` (optionnel, copié depuis `.env.example`) permet d'activer SMTP/webhook sans modifier le compose.

## Configuration

Toutes les variables sont préfixées `RUSTMON_` et se chargent depuis `.env` ou l'environnement du process — un redémarrage est nécessaire pour appliquer un changement. Voir [`.env.example`](.env.example) pour la liste complète et [`specification.md#5-configuration`](specification.md) pour le détail de chaque variable.

Principales :

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `RUSTMON_BIND_ADDR` | `0.0.0.0:3000` | Adresse d'écoute HTTP |
| `RUSTMON_DB_PATH` | `data.db` | Chemin du fichier SQLite |
| `RUSTMON_DISK_PATH` | `/` | Mount point scanné pour les métriques disque (`/host` en Docker) |
| `RUSTMON_SAMPLE_INTERVAL_SECS` | `5` | Fréquence de collecte |
| `RUSTMON_RETENTION_DAYS` | `7` | Rétention de l'historique |
| `RUSTMON_*_THRESHOLD_PCT` | `90.0` | Seuils d'alerte CPU/RAM/disque |

## Développement

```bash
cargo build          # build debug
cargo test            # tests
cargo clippy          # lint
```

Prérequis : Rust édition 2024 (rustc ≥ 1.85). Pour le monitoring Docker, l'utilisateur exécutant le binaire doit avoir accès à `/var/run/docker.sock` (groupe `docker` ou équivalent).

## Architecture

| Composant | Crate | Rôle |
| --- | --- | --- |
| Serveur web / SSE | `axum`, `tokio` | HTTP async + flux SSE `/api/stream` |
| Frontend | HTMX, Pico CSS | DOM mis à jour par fragments HTML reçus en SSE |
| Templates | `askama` | Rendu SSR des fragments |
| Assets | `rust-embed` | HTML/CSS embarqués dans le binaire |
| Base de données | SQLite (`sqlx`) | Historique métriques + alertes |
| Collecte VPS | `sysinfo` | CPU/RAM/disque hôte |
| Collecte Docker | `bollard` | Découverte + stats des conteneurs |
| Config | `serde`, `envy`, `dotenvy` | `.env` → struct `Config` typée |
| Notifications | `lettre` (SMTP), `reqwest` (webhook) | Alertes e-mail / webhook |
