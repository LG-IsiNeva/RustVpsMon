# RustVpsMon

## 1. Présentation du Projet

**Nom du projet :** RustVpsMon
**Objectif :** Développer un agent de monitoring "tout-en-un" ultra-léger sous forme d'un binaire unique. Il doit surveiller les ressources d'un VPS hôte, découvrir et monitorer dynamiquement les conteneurs Docker présents, stocker l'historique localement, et exposer un tableau de bord web temps réel sans dépendance frontend lourde.

---

## 2. Architecture Technique

La pile technologique est conçue pour maximiser les performances tout en minimisant l'empreinte mémoire et la complexité de déploiement.

| Composant | Technologie / Crate Rust | Rôle et justification |
| --- | --- | --- |
| **Serveur Web & API** | `axum`, `tokio` | Serveur HTTP asynchrone performant pour exposer l'interface et gérer les connexions SSE persistantes. |
| **Frontend** | HTMX, HTML pur, CSS (Pico CSS) | Interface sans JavaScript complexe. Mise à jour du DOM via les fragments HTML reçus par SSE. |
| **Moteur de Template** | `askama` | Génération côté serveur (SSR) des fragments HTML à envoyer via SSE. |
| **Assets** | `rust-embed` | Embarque les fichiers HTML/CSS statiques directement dans le binaire compilé. |
| **Base de Données** | SQLite (`sqlx`) | Stockage local léger. `sqlx` permet des requêtes asynchrones vérifiées à la compilation. |
| **Collecte VPS** | `sysinfo` | Lecture cross-platform et native des métriques (CPU, RAM, Disque) du système hôte. |
| **Collecte Docker** | `bollard` | Client API Docker asynchrone pour la découverte et la lecture du flux `stats` des conteneurs. |
| **Configuration** | `serde` + `envy` + `dotenvy` | Chargement d'un fichier `.env` puis désérialisation des variables d'environnement (préfixées `RUSTMON_`) dans une struct `Config` typée. |
| **Envoi d'e-mails** | `lettre` | Client SMTP asynchrone (pas d'API tierce) pour les notifications d'alerte. |
| **Webhooks** | `reqwest` | Envoi de payloads JSON vers des URLs externes (Discord, Slack, Gotify, etc.). |

---

## 3. Spécifications Fonctionnelles

### Tâche 1 : Collecte VPS (Hôte)

* Interroger l'utilisation globale du CPU (en pourcentage).
* Interroger l'utilisation de la mémoire RAM (utilisée / totale).
* Interroger l'espace disque (utilisé / total).
* Fréquence : Toutes les N secondes (ex: 5 secondes).
* Action : Insérer ces données en base avec un horodatage.

### Tâche 2 : Découverte et Monitoring Docker

* **Découverte :** Au démarrage (et périodiquement), lister les conteneurs Docker en cours d'exécution via le socket Unix (`/var/run/docker.sock`).
* **Monitoring :** S'abonner au flux d'événements/statistiques de chaque conteneur actif pour récupérer son utilisation CPU et RAM.
* **Cycle de vie :** Gérer dynamiquement l'apparition de nouveaux conteneurs et la disparition des conteneurs arrêtés pour éviter les fuites de mémoire.
* Action : Insérer ces données en base, associées à l'ID ou au nom du conteneur.

### Tâche 3 : Interface Web (Frontend)

* Afficher les métriques actuelles du VPS (Jauges ou textes mis à jour en temps réel).
* Afficher une liste dynamique des conteneurs actifs avec leurs métriques respectives.
* Le client s'abonne à un unique endpoint SSE (`/api/stream`).
* Le client reçoit directement du HTML pré-formaté par Rust et l'injecte dans les balises correspondantes (grâce à HTMX : `hx-ext="sse"` et `sse-swap`).

---

## 4. Spécifications Techniques

### Modèle de Données (SQLite)

Deux tables principales pour gérer l'historique (pour de futurs graphiques ou calculs de moyennes) :

```sql
CREATE TABLE vps_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    cpu_usage REAL NOT NULL,
    ram_usage REAL NOT NULL,
    disk_usage REAL NOT NULL
);

CREATE TABLE docker_metrics (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    container_name TEXT NOT NULL,
    cpu_usage REAL NOT NULL,
    ram_usage REAL NOT NULL
);

```

### Mécanisme de Synchronisation (Backend vers Frontend)

1. Les tâches de collecte (VPS et Docker) tournent dans des processus `tokio::spawn`.
2. Ces tâches envoient les métriques brutes vers un canal de diffusion (broadcast channel : `tokio::sync::broadcast`).
3. Le routeur SSE (`Axum`) écoute ce canal.
4. À chaque nouvelle donnée, `Axum` passe les valeurs dans un template HTML (via `Askama`), génère un fragment (ex: `<div id="vps-cpu">15%</div>`), et l'envoie dans le flux SSE.

---
## Système d'Alertes Évolué

### 1. Pile Technique Complémentaire

| Composant | Technologie / Crate Rust | Rôle et justification |
| --- | --- | --- |
| **Envoi d'e-mails** | `lettre` | Envoi via un serveur SMTP standard (voir section Configuration) — pas de dépendance à une API tierce. |
| **Webhooks** | `reqwest` | Envoi de payloads JSON vers des URLs externes (Discord, Slack, Gotify, etc.). |

---

### 2. Spécifications Fonctionnelles

#### 2.1. Détection et Anti-Spam (Dé-duplication)

Pour éviter d'envoyer 100 alertes si le CPU reste à 99% pendant 2 heures, l'application doit implémenter un mécanisme d'**état d'alerte**.

* **Règle de déclenchement :** Une alerte est créée lorsque le seuil critique est dépassé (ex: `CPU > 90%` ou `Conteneur X arrêté`).
* **Contrainte d'unicité :** Tant qu'une alerte sur un composant précis est active (non résolue / non acquittée), **aucun nouvel e-mail ou webhook ne peut être envoyé** pour ce même motif.
* **Notification de retour à la normale (Optionnel mais recommandé) :** Lorsque la métrique repasse sous le seuil, le système envoie une notification unique "Incident résolu" et ferme l'alerte.

#### 2.2. Gestion et Acquittement (Interface HTMX)

* **Visualisation :** Une cloche (rouge + nombre d'alertes non acquittées, ou verte + "Aucune alerte") est affichée en permanence. Un clic ouvre une modale (Alpine.js) listant les alertes actives dans un tableau.
* **Acquittement :** Dans la modale, chaque ligne `TRIGGERED` a un bouton "Acquitter" (géré via `hx-post="/api/alerts/{id}/acknowledge"`).
* **Effet de l'acquittement :** L'alerte passe dans l'état "Acquittée". Le compteur de la cloche décroît (seules les alertes `TRIGGERED` sont comptées), et le système est de nouveau autorisé à renvoyer une notification si l'incident se reproduit à l'avenir.

---

### 3. Spécifications Techniques

#### 3.1. Évolution de la Base de Données (SQLite)

Une nouvelle table est requise pour suivre l'état des alertes.

```sql
CREATE TABLE alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
    component TEXT NOT NULL,          -- ex: "vps_cpu", "vps_disk", "docker_container_nginx"
    message TEXT NOT NULL,            -- ex: "CPU usage at 95%"
    status TEXT NOT NULL,             -- 'TRIGGERED' (En cours), 'ACKNOWLEDGED' (Acquittée), 'RESOLVED' (Résolue)
    acknowledged_at DATETIME,
    resolved_at DATETIME
);

```

#### 3.2. Logique algorithmique dans la tâche de collecte Rust

À chaque cycle de collecte (ex: toutes les 5 secondes) :

1. **Vérification du seuil :** La métrique dépasse-t-elle le seuil ? (Ex: RAM > 90%).
2. **Vérification de l'état en Base :** Existe-t-il déjà une alerte avec le `component = 'vps_ram'` et le `status = 'TRIGGERED'` ?
* **OUI :** Ne rien faire (l'incident est déjà connu, le webhook/mail a déjà été envoyé au premier déclenchement).
* **NON :** 1. Insérer la nouvelle alerte avec le statut `TRIGGERED` en base SQLite.
2. Déclencher un thread asynchrone (`tokio::spawn`) pour envoyer l'e-mail (SMTP) et/ou le Webhook.
3. Pousser la mise à jour via le flux SSE pour afficher l'alerte instantanément sur le front HTMX.

---

### 4. Format des Notifications (Payloads)

#### Discord / Slack Webhook (JSON)

```json
{
  "content": "⚠️ **Alerte Serveur** : Le conteneur `production_api` s'est arrêté de manière inattendue."
}

```

#### E-mail (SMTP)

L'e-mail est envoyé via `lettre` en se connectant directement au serveur SMTP configuré (`RUSTMON_SMTP_HOST`/`RUSTMON_SMTP_PORT`, authentifié avec `RUSTMON_SMTP_USER`/`RUSTMON_SMTP_PASSWORD` si fournis), avec `RUSTMON_EMAIL_FROM` comme expéditeur et `RUSTMON_EMAIL_TO` (liste séparée par des virgules) comme destinataires. Le corps est envoyé en HTML, par exemple :

```html
<strong>Le VPS subit une charge CPU critique (96%).</strong><br>Connectez-vous au dashboard pour acquitter l'alerte.
```

---

## 5. Configuration

La configuration est chargée une seule fois au démarrage depuis un fichier `.env` (voir `.env.example` à la racine) et/ou les variables d'environnement du process, toutes préfixées `RUSTMON_`. Un redémarrage est nécessaire pour appliquer un changement — il n'y a pas de rechargement à chaud ni d'interface de configuration web.

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `RUSTMON_CPU_THRESHOLD_PCT` | `90.0` | Seuil d'alerte CPU (%) |
| `RUSTMON_RAM_THRESHOLD_PCT` | `90.0` | Seuil d'alerte RAM (%) |
| `RUSTMON_DISK_THRESHOLD_PCT` | `90.0` | Seuil d'alerte disque (%) |
| `RUSTMON_SAMPLE_INTERVAL_SECS` | `5` | Fréquence de collecte des métriques |
| `RUSTMON_RETENTION_DAYS` | `7` | Rétention de l'historique en base |
| `RUSTMON_DB_PATH` | `data.db` | Chemin du fichier SQLite |
| `RUSTMON_BIND_ADDR` | `0.0.0.0:3000` | Adresse d'écoute HTTP |
| `RUSTMON_SMTP_HOST` | _(absent)_ | Hôte SMTP — l'envoi d'e-mail est désactivé si absent |
| `RUSTMON_SMTP_PORT` | `587` | Port SMTP |
| `RUSTMON_SMTP_USER` / `RUSTMON_SMTP_PASSWORD` | _(absent)_ | Identifiants SMTP, optionnels |
| `RUSTMON_EMAIL_FROM` | _(absent)_ | Adresse expéditrice — requise pour activer l'e-mail |
| `RUSTMON_EMAIL_TO` | _(absent)_ | Destinataires, séparés par des virgules |
| `RUSTMON_WEBHOOK_URL` | _(absent)_ | URL de webhook (Discord/Slack/...) — désactivé si absent |

---

## 6. Contraintes et Sécurité

* **Fichier Unique :** Le livrable final doit être un unique exécutable (ex: `rustmon`). La base SQLite sera générée dynamiquement au premier lancement dans le dossier d'exécution (ex: `data.db`).
* **Droits d'accès :** Le binaire Rust nécessitera l'accès au socket Docker. Sur un VPS Linux, l'utilisateur exécutant le programme devra faire partie du groupe `docker` ou le programme devra être lancé avec les droits suffisants.
* **Purge des données :** La base SQLite enregistrant des données toutes les 5 secondes, une tâche de rotation ou de nettoyage des données vieilles de plus de X jours devra être implémentée pour éviter la saturation du disque.
* **Empreinte :** L'application ne doit pas dépasser 20-30 Mo de RAM en vitesse de croisière.