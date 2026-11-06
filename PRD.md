# PRD — Ferrix Explorer

> Explorateur de fichiers open-source, natif Windows 10/11, écrit en Rust.

> **Document fusionné (v3.0).** Ce PRD fusionne trois PRD concurrents en une source de vérité unique :
> - `PRD_claude-haiku-5.5-high.txt` — **socle** : backlog traçable (IDs), critères d'acceptation, décisions légales, threading, personas, process.
> - `PRD_gpt-5.6-sol-medium.txt` — **exigences et ingénierie** : spécification fonctionnelle détaillée, sécurité, tests, KPIs.
> - `PRD_claude-sonnet-5-high.txt` — **synthèse** : cadrage « killer features », matrice par thème, arbitrages de roadmap.
>
> Les divergences entre les trois ont été arbitrées et sont actées en **section 6**. Les contributions de chaque document sont tracées en **Annexe C**.

---

## 0. Informations du document

| Champ | Valeur |
|---|---|
| Nom de code | Ferrix Explorer (provisoire, à valider — voir 9.4) |
| Version du document | 3.0 (fusion) |
| Statut | Draft — à valider avant Tranche 0 |
| Type de produit | Application desktop native Windows |
| Licence cible | MIT OU Apache-2.0 (sous réserve de la décision UI, section 9.1) |
| Plateformes | Windows 10 (22H2+) et Windows 11 (x64 prioritaire, ARM64 en stretch) |
| Langage principal | Rust — positionnement **Rust-first** (voir 8.1) |
| Distribution | MSI, MSIX et version portable |
| Dépôt | Monorepo Cargo multi-crates |
| Langue du projet | Français (documentation), anglais (code, commentaires techniques) |
| Recherche sémantique / IA | **Hors périmètre, définitivement** (5.3) |
| Remplacement du shell `explorer.exe` | **Hors périmètre de la 1.0** (5.3) |

---

## 1. Résumé exécutif

Ferrix Explorer est une alternative open source à l'Explorateur de fichiers Windows : une application rapide au démarrage, fluide sur les dossiers massifs, réactive pendant les opérations longues, compatible avec les comportements Windows habituels, intégrée au Shell, dotée d'une recherche globale instantanée, accessible au clavier et avec les lecteurs d'écran, extensible sans compromettre stabilité ni sécurité, et respectueuse de la vie privée.

L'application doit couvrir les usages principaux de l'Explorateur Windows :

- navigation dans les dossiers (locaux, UNC, WSL, réseau, cloud) ;
- affichage en liste, détails, grille, mosaïque et grandes icônes ;
- opérations de copie, déplacement, suppression et renommage ;
- menu contextuel Windows complet avec extensions tierces ;
- miniatures et aperçus (Quick Look par barre d'espace) ;
- onglets, fenêtres multiples et mode dual-pane ;
- sidebar complète (épinglés, accès rapide, récents, lecteurs, cloud) ;
- recherche globale instantanée façon Everything ;
- raccourcis clavier avec parité Explorer ;
- drag-and-drop interne et OLE ;
- propriétés et métadonnées ;
- corbeille, archives, terminal intégré ;
- fonctionnalités avancées de gestion des fichiers.

**Dix raisons concrètes de quitter l'Explorateur Windows :** une navigation très fluide ; une recherche globale instantanée ; des onglets et panneaux avancés ; un aperçu rapide à la barre d'espace ; une palette de commandes ; un terminal intégré ; une gestion robuste des opérations ; un mode dual-pane ; des outils d'analyse et d'organisation ; une architecture ouverte aux contributions.

**Positionnement Rust-first :** toute la logique métier du projet est écrite en Rust. Certaines fonctionnalités nécessiteront des API Windows, du COM, des DLL système ou des bibliothèques tierces optionnelles. L'objectif n'est pas de garantir qu'aucun code natif externe ne soit chargé, mais que le code du projet soit en Rust et que les dépendances soient isolées, auditées et documentées (section 10.6).

L'objectif « 1000 fois plus rapide » est une ambition marketing, pas un critère technique littéral : **chaque gain de performance annoncé doit être prouvé par un benchmark reproductible** face à `explorer.exe` (sections 5.2 et 14.1).

---

## 2. Vision produit

> Construire l'explorateur de fichiers Windows open source le plus rapide, le plus fluide et le plus productif, sans sacrifier la compatibilité avec le Shell Windows, l'accessibilité ou la sécurité des données.

Ferrix Explorer vise la parité fonctionnelle avec l'Explorateur Windows, l'enrichit de fonctionnalités de power-user (aperçu instantané façon Quick Look, palette de commandes, terminal intégré, recherche instantanée façon Everything) et reste nettement plus réactif sur les dossiers volumineux.

L'ambition est formulée en **objectifs mesurables (section 5.2)** plutôt qu'en multiplicateur marketing.

---

## 3. Problème adressé

**Explorer Windows aujourd'hui :**

- Latence visible sur dossiers volumineux (dizaines de milliers d'entrées) et sur les partages réseau.
- Recherche intégrée lente, dépendante de l'indexeur Windows Search.
- Pas d'aperçu rapide natif (équivalent de Quick Look sur macOS).
- Extensibilité limitée, UI évoluant lentement, code source fermé.

**Alternatives existantes et leurs limites :**

- **Total Commander** : payant, interface datée.
- **Files** (C#/WinUI, MIT) : bonne référence fonctionnelle, mais pas en Rust et performances variables.
- **Double Commander** : multiplateforme, look non natif Windows 11.
- **Everything** : excellent pour la recherche, mais ce n'est pas un explorateur complet.

**Opportunité :** un explorateur natif Windows 11 (Mica, Snap Layouts), rapide, open-source, avec les fonctions power-user réunies dans une seule application.

---

## 4. Principes directeurs

### 4.1 Performance mesurable

- Ne jamais bloquer le thread UI avec une opération disque, réseau ou Shell.
- Virtualiser toutes les longues listes.
- Charger les miniatures uniquement lorsqu'elles sont visibles ou bientôt visibles.
- Annuler les tâches devenues inutiles après un changement de dossier.
- Utiliser des caches limités par une politique LRU et un budget mémoire.
- Mesurer avant d'optimiser ; publier les scénarios et résultats de benchmarks.

### 4.2 Sécurité des données

- Ne jamais considérer une opération réussie avant confirmation du système.
- Préférer la Corbeille à la suppression définitive ; exiger une confirmation explicite pour toute action destructrice.
- Ne jamais suivre silencieusement des liens ou reparse points dangereux pendant une opération récursive.
- Prévoir la reprise, l'annulation et le reporting détaillé des erreurs ; ne jamais prétendre qu'une opération est réversible lorsqu'elle ne l'est pas.
- Ne jamais journaliser les contenus de fichiers, tokens ou chemins sensibles sans consentement.

### 4.3 Intégration Windows

Utiliser les API Windows officielles quand elles apportent une meilleure compatibilité : Windows Shell, COM, `IFileOperation`, `IContextMenu`, OLE Drag-and-Drop, Known Folders, Media Foundation, DirectWrite, UI Automation, Cloud Files API, USN Journal, Windows Search en fallback éventuel. Fournir un fallback lorsque l'API native est indisponible ou instable.

### 4.4 Accessibilité dès la conception

- Navigation complète au clavier, focus visible, noms/rôles/états accessibles.
- Compatibilité Narrator et UI Automation, contraste élevé.
- Respect des paramètres d'animation de Windows.
- **Ne pas reporter l'accessibilité à la fin du projet** (voir 6, décision D-3).

### 4.5 Open source et licences maîtrisées

- Le build par défaut n'utilise que des composants dont la redistribution est compatible avec la licence du projet.
- Toute dépendance restrictive est : optionnelle, désactivée par défaut, isolée, documentée, et acceptée explicitement par l'utilisateur.
- Inventaire SBOM produit à chaque release (section 19.2).

---

## 5. Objectifs et non-objectifs

### 5.1 Objectifs produit

- Remplacer l'Explorateur Windows pour les opérations quotidiennes, sans rompre les conventions Windows.
- Navigation fluide : listes virtualisées, zéro lag sur dossiers massifs (fluidité visée jusqu'à 100 000 éléments).
- Menu contextuel **natif Windows** complet, y compris entrées des applications tierces (7-Zip, Git, antivirus).
- Recherche globale instantanée via lecture de l'USN Journal / MFT NTFS, sur plusieurs millions de fichiers.
- Multi-onglets, multi-fenêtres, split view et mode dual-pane façon Total Commander.
- Quick Look : aperçu instantané de la plupart des formats courants.
- Sidebar complète : épinglés, accès rapide, récents, bibliothèques, lecteurs, réseau, comptes cloud.
- Intégration cloud (OneDrive, Google Drive, Dropbox) via les API de synchronisation Windows, sans réimplémentation.
- Palette de commandes, terminal intégré, CLI compagnon, support WSL.
- Accessibilité complète dès le départ (UI Automation, navigation clavier intégrale, contraste élevé).
- Architecture extensible par plugins sandboxés (WASM) en v2.
- Préserver sessions, onglets et opérations après un crash ; utilisation locale sans compte ni télémétrie obligatoire.

**Objectifs techniques :** architecture multi-crates modulaire ; séparation stricte UI / domaine / accès système / infrastructure ; modèle asynchrone avec annulation ; couverture de tests élevée sur la logique critique ; interop Windows confinée dans des crates dédiées ; benchmarks reproductibles ; packaging installable et portable ; x64 en priorité, ARM64 ensuite.

### 5.2 Objectifs de performance (mesurables)

Toutes les mesures se font sur une machine de référence définie dans `docs/benchmarks/REFERENCE.md` (créée en Tranche 0), avec comparaison systématique à `explorer.exe` dans les mêmes conditions.

**Arbitrage (décision D-2, section 6)** : les cibles de v1.0 reprennent les valeurs **exigeantes** du PRD haiku ; les valeurs plus réalistes du PRD gpt-5.6 sont le **plancher acceptable** — un écart documenté et accepté peut être acté, jamais dépassé silencieusement.

| Métrique | Cible v1.0 | Cible stretch | Plancher acceptable |
|---|---|---|---|
| Démarrage à froid (fenêtre affichée et interactive) | < 300 ms | < 200 ms | < 500 ms |
| Démarrage chaud (fenêtre affichée) | < 200 ms | < 150 ms | < 200 ms |
| Feedback visuel après navigation | < 50 ms | < 30 ms | < 50 ms |
| Première vue d'un dossier de 50 000 entrées | < 100 ms | < 50 ms | < 150 ms |
| Dossier de 100 000 éléments | UI interactive pendant le chargement | idem | idem |
| Scroll sur 50 000 entrées | 60 FPS constant | 120 FPS sur écran compatible | 60 FPS |
| Navigation vers un dossier déjà visité (cache chaud) | < 50 ms | < 20 ms | < 50 ms |
| Changement d'onglet | < 50 ms | < 30 ms | < 50 ms |
| Recherche par nom sur index de 1 M de fichiers (p95) | < 50 ms | < 20 ms | < 100 ms |
| Indexation initiale de 1 M de fichiers (NTFS, USN/MFT) | < 60 s | < 30 s | sans blocage UI |
| Mémoire au repos, 1 onglet, dossier vide | < 120 Mo | < 80 Mo | < 150 Mo |
| Taille de l'installeur sans moteurs optionnels | < 30 Mo | < 20 Mo | — |
| Ouverture de l'aperçu Quick Look (image standard) | < 100 ms | < 50 ms | < 150 ms |
| Première page PDF affichée | < 300 ms | < 200 ms | — |

Ces valeurs sont des objectifs mesurables, pas des garanties contractuelles. Toute régression supérieure à 10 % sur un scénario de benchmark bloque la fusion (CI, scénarios documentés — voir 14.1).

### 5.3 Non-objectifs (v1.0)

- Remplacement du process `explorer.exe` (le shell Windows reste intact ; Ferrix est standalone, avec option « explorateur par défaut » en v1.5, sans hacks fragiles de registre).
- Remplacement de la barre des tâches ou du bureau.
- Support Linux/macOS livré (l'architecture reste modulaire pour un portage futur, aucun travail avant la 1.0).
- Recherche sémantique ou IA. **Hors scope, définitivement.**
- Éditeur de texte, d'images, de vidéos, de PDF ou suite bureautique. On s'appuie sur les applications existantes et sur Quick Look.
- Client de synchronisation cloud propriétaire : on s'intègre aux clients officiels, on ne réimplémente pas la synchronisation.
- Antivirus ou outil de sécurité : on s'appuie sur le scan natif via le Shell.
- Garantie de compatibilité avec toutes les extensions Shell tierces (la parité exacte des menus Windows 10/11 n'est pas garantie — voir 13.8).
- Inclusion par défaut de codecs ou moteurs dont la licence est incompatible.
- Promesse absolue de performance du type « 1000x » sans mesure.

---

## 6. Décisions actées (arbitrations entre les PRD d'origine)

Ce chapitre tranche les divergences entre les trois PRD fusionnés. Toute évolution ultérieure passe par un ADR.

| # | Sujet | Décision | Justification |
|---|---|---|---|
| **D-1** | Priorité de Quick Look | **MVP (Tranche 8)** — le PRD sonnet le repoussait en v1.5 | 2 voix contre 1 ; c'est une killer feature (impact 5) et la tranche dédiée existe déjà |
| **D-2** | Budgets de performance | **Table de 5.2** : cibles exigeantes (haiku) + plancher acceptable (gpt-5.6) | Évite aussi les cibles contradictoires (< 300 ms vs < 500 ms de démarrage) |
| **D-3** | Positionnement de l'accessibilité | **Continué dès la Tranche 0**, audit complet Tranche 10 | Le PRD sonnet a raison : ajoutée après coup sur une UI figée, elle est beaucoup plus coûteuse ; le PRD haiku l'avait déjà comme objectif transverse |
| **D-4** | Roadmap | **19 tranches (table 16.1)** : Quick Look + compression fusionnés en Tranche 8 (haiku), a11y complète en Tranche 10 avec jalons à chaque tranche (gpt) | Compromis entre les découpages haiku et gpt ; le PRD gpt reste la référence pour la sécurité des archives (12.5) |
| **D-5** | Découpage des crates | **Découpage fin** : `fx-domain` / `fx-fs` / `fx-ops` / `fx-storage` distincts (gpt) plutôt qu'un `fx-core` monolithique (haiku) | Réduit le couplage et les temps de build ; `fx-core` devenait une dépendance de tout |
| **D-6** | Raccourci de la palette de commandes | `Ctrl+Maj+P` (= `Ctrl+Shift+P`) | Cosmétique : même touche selon la langue de saisie |
| **D-7** | Positionnement « pure Rust » | **Rust-first transparent** : le code du projet est en Rust, l'interop Windows est confinée et documentée, aucune promesse d'absence totale de code natif | Honnête vis-à-vis des contributeurs et de `cargo deny` |
| **D-8** | Recherche IA / sémantique | Exclue du scope — **définitivement**, pas « plus tard » | Acté par les trois PRD |
| **D-9** | Nom du projet | « Ferrix » maintenu comme nom de code provisoire, vérifié avant première publication | Voir 9.4 |

**Décisions déjà présentes dans les PRD d'origine et conservées :** accessibilité anticipée ; Quick Look en tranche dédiée ; `fx-preview` comme crate dédié avec providers pluggables ; LZTurbo jamais inclus par défaut ; UnRAR en lecture seule derrière une feature flag ; télémétrie strictement opt-in.

---

## 7. Personas et utilisateurs cibles

### 7.1 Matrice des personas

| Persona | Besoins principaux | Fonctionnalités clés |
|---|---|---|
| **Développeur** | Terminal, Git, WSL, chemins longs, navigation clavier | Palette de commandes, terminal ConPTY, badges Git, support WSL, CLI `fx` |
| **Power user / archiviste** | Traitements en masse, comparaison, intégrité | Dual-pane, renommage par lot regex, checksums, détecteur de doublons, file d'attente |
| **Utilisateur grand public** | Simplicité, look moderne, aperçu rapide | Thèmes, Mica, Quick Look, navigation souris |
| **Lecteur d'écran** | Accès complet sans souris ni vision | UI Automation, annonces cohérentes, navigation clavier intégrale |
| **Administrateur / support** | Déploiement, diagnostic, usage sans installation | Mode portable, logs `tracing`, CLI, export de configuration |

### 7.2 Besoins détaillés

**Utilisateur standard** — retrouver ses documents ; copier, déplacer, supprimer, renommer ; naviguer dans Téléchargements, Documents, Images ; accéder à OneDrive ; interface familière.

**Power user** — onglets ; dual-pane ; renommage par lot ; recherche avancée ; archives ; checksums ; comparaison de dossiers ; file d'attente d'opérations ; raccourcis personnalisés.

**Développeur** — terminal intégré ; Git ; WSL ; CLI ; chemins longs ; fichiers cachés ; accès rapide à PowerShell, CMD et Windows Terminal ; navigation rapide dans de grands dépôts.

**Administrateur système** — SMB et lecteurs réseau ; permissions NTFS ; élévation ponctuelle ; checksums ; volumes amovibles ; logs techniques ; opérations robustes.

**Accessibilité** — lecteur d'écran ; navigation 100 % clavier ; contraste élevé ; focus prévisible ; annonces des opérations ; interface utilisable sans drag-and-drop.

---

## 8. Stack technique

### 8.1 Choix techniques

| Composant | Choix | Licence | Notes |
|---|---|---|---|
| Langage | Rust (edition 2024 si stable à la Tranche 0, sinon 2021) | — | MSRV fixée dans `rust-toolchain.toml` |
| UI | Slint (plan B : iced, ou Win32 + Direct2D) | GPLv3 / Royalty-Free / commerciale | **Décision bloquante, voir 9.1** |
| Interop Windows | `windows-rs` (IShellFolder, IContextMenu, IFileOperation, CfApi, ConPTY, UIA, DirectWrite, Media Foundation) | MIT/Apache-2.0 | API officielle Microsoft |
| Parcours FS | `jwalk` + `rayon` | MIT / MIT-Apache | Parcours parallèle |
| Surveillance FS | `notify` | Permissive | Événements de dossiers ouverts |
| Index et recherche | `tantivy` + lecture USN Journal / MFT | MIT | Recherche façon Everything |
| Base locale | `rusqlite` (SQLite embarqué) | MIT | Métadonnées, récents, tags, config (voir 10.9) |
| Async | `tokio` | MIT | I/O réseau, cloud, orchestration |
| Cloud | `reqwest`, `oauth2`, Microsoft Graph, Google Drive API v3, Dropbox API | MIT/Apache-2.0 | OAuth2 + PKCE via navigateur système |
| Compression (défaut) | `libdeflater`, `zstd`, `sevenz-rust` | MIT / BSD / Apache-2.0 | Voir 12 |
| Compression (RAR, lecture) | `unrar` via `unrar.dll` | **Restrictive** | Optionnel, désactivable, voir 9.3 |
| Images | `zune-image`, `fast_image_resize` | MIT/Apache-2.0 | Décodage et redimensionnement SIMD |
| RAW | `rawloader` ou successeur maintenu | MIT/Apache-2.0 | À confirmer à la Tranche 8 (maintenance) |
| PDF | `pdfium-render` (moteur Pdfium) | BSD / MIT-Apache | **Pas de MuPDF** (AGPL) |
| Audio | `symphonia` | MPL-2.0 | Waveform, métadonnées |
| Vidéo | Windows Media Foundation (natif) | Système | `ffmpeg` en option (plugin) |
| Coloration syntaxique | `tree-sitter` (fallback `syntect`) | MIT | Incrémental |
| Polices | DirectWrite (natif) | Système | Aperçu de polices |
| Terminal | ConPTY via `windows-rs` | MIT/Apache-2.0 | Terminal intégré |
| Config | `serde` + TOML | MIT/Apache-2.0 | Fichiers utilisateur |
| Logs | `tracing` + `tracing-subscriber` | MIT | Rotation, redaction (voir 14.5) |
| Plugins (v2) | `wasmtime` | Apache-2.0 avec exception LLVM | Sandbox WASM |
| Packaging | `cargo-wix` (MSI) et MSIX | MIT | Installeur et Store-ready |
| CI/CD | GitHub Actions (`windows-latest`) | — | Build, tests, lint, licences, release |

### 8.2 Exigences de sélection d'une dépendance

Chaque dépendance est examinée selon : licence ; activité du projet ; vulnérabilités ; présence de code natif ; compatibilité Windows ARM64 ; taille ; politique de mise à jour ; nécessité réelle. Tout composant sensible (LZTurbo, RAR, FFmpeg, PDFium, SDK divers) fait l'objet d'un ADR et d'une vérification juridique avant distribution (voir 9 et 19.2).

---

## 9. Décisions ouvertes et points légaux

### 9.1 Licence de l'UI (décision bloquante avant Tranche 0)

Slint est disponible sous GPLv3, sous licence Royalty-Free (obligation d'afficher un badge « Made with Slint », licence non OSI) ou sous licence commerciale. **Une licence MIT/Apache-2.0 pour Ferrix est incompatible avec une dépendance GPLv3 distribuée dans le binaire.**

**Options :**

1. Licence du projet en **GPLv3** (simple, OSI, compatible Slint). Renonce à la double licence MIT/Apache-2.0.
2. Garder **Slint Royalty-Free** et accepter le badge et une licence non OSI pour la partie UI.
3. Passer à **iced** (MIT) ou à **Win32 + Direct2D** (contrôle total, développement plus long).

**Recommandation :** trancher en Tranche 0 au moyen des prototypes (section 11), et ne retenir Slint **que s'il satisfait les critères d'accessibilité et d'intégration Windows**. Critères du spike : virtualisation de 100 000 éléments, UI Automation, drag-and-drop OLE, menus natifs, rendu DPI, IME, multi-fenêtres, Snap Layouts, Mica, latence, mémoire, taille du binaire, licence. Documenter dans `docs/adr/0001-ui-framework.md`.

### 9.2 LZTurbo (powturbo)

Licence non OSI (usage gratuit non commercial, source non libre). Il ne sera donc **jamais inclus dans le build distribué par défaut**. C'est un **backend optionnel** que l'utilisateur installe lui-même et active explicitement, avec avertissement affiché dans les paramètres. Avant toute prise en compte : vérifier licence actuelle à la date d'intégration, droits de redistribution, usage commercial, disponibilité des sources, compatibilité x64/ARM64, performances sur scénarios réels, audit de sécurité.

Cette logique **« défaut OSS + moteur tiers opt-in »** s'applique à tout composant non permissif (MuPDF, ffmpeg en build complet, etc.).

### 9.3 UnRAR

La licence d'UnRAR interdit la reconstruction de l'algorithme RAR. Le support RAR est limité à la **lecture et l'extraction** via `unrar.dll` officielle, isolée dans le crate `fx-codec`, désactivable à la compilation (feature flag `rar`) et documentée dans `docs/LEGAL.md`. Pas de création d'archives RAR.

### 9.4 Nom du projet

« Ferrix » doit être vérifié (collisions de marques, noms de domaine, dépôts GitHub, crates.io) avant la première publication.

### 9.5 Télémétrie et vie privée

Strictement **opt-in**, transparente (données listées dans les paramètres), anonyme, désactivable à tout moment. Aucune télémétrie par défaut, aucun compte requis, aucun chemin de fichier envoyé par défaut, crash reporting opt-in avec prévisualisation avant envoi, recherches locales. Détail en section 14.4.

### 9.6 Licences et dépendances (livraison)

Le projet maintient `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `docs/LEGAL.md` et un **SBOM par release**. Toute dépendance non permissive est listée dans `docs/LEGAL.md` avec son statut : **par défaut**, **optionnelle**, **isolée** ou **opt-in**. `cargo deny` bloque en CI toute licence non autorisée sans exception documentée.

---

## 10. Architecture logicielle

### 10.1 Workspace Cargo (monorepo multi-crates)

Découpage fin retenu (décision D-5) :

```text
ferrix-explorer/
├── Cargo.toml                  # workspace
├── rust-toolchain.toml         # MSRV figée
├── CLAUDE.md                   # règles pour les agents IA (voir 20.1)
├── crates/
│   ├── fx-domain/              # Entités, règles métier, commandes (FsEntry, règles de sélection…)
│   ├── fx-fs/                  # Listing, métadonnées, chemins longs, parcours, surveillance
│   ├── fx-ops/                 # File d'opérations, conflits, undo/redo, checksums
│   ├── fx-shell/               # Interop COM Shell (IContextMenu, IFileOperation, icônes, propriétés)
│   ├── fx-index/               # Recherche : USN/MFT reader + tantivy
│   ├── fx-preview/             # Quick Look : providers d'aperçu
│   ├── fx-codec/               # Backends compression/décompression pluggables
│   ├── fx-cloud/               # Providers cloud (trait CloudProvider) : onedrive/, gdrive/, dropbox/
│   ├── fx-network/             # SMB, FTP/SFTP/WebDAV (v2)
│   ├── fx-terminal/            # Terminal intégré (ConPTY)
│   ├── fx-git/                 # États Git, badges (v1.5)
│   ├── fx-a11y/                # Accessibilité (wrapper UIA, annonces, rôles)
│   ├── fx-settings/            # Config TOML, thèmes, raccourcis, persistance, mode portable
│   ├── fx-storage/             # SQLite, migrations, caches persistants
│   ├── fx-ui/                  # Composants UI (vues, sidebar, onglets, palette)
│   ├── fx-plugin-api/          # API stable pour plugins (v2, squelette vide en 1.0)
│   ├── fx-plugin-host/         # Runtime WASM (v2, squelette vide en 1.0)
│   ├── fx-cli/                 # Binaire CLI compagnon `fx`
│   └── fx-app/                 # Binaire principal, orchestration
├── resources/                  # Icônes, assets, traductions
├── benchmarks/                 # Scénarios et résultats de benchmarks
├── test-data/                  # Jeux de données de test
├── installer/                  # Scripts cargo-wix, manifest MSIX
├── scripts/
└── docs/
    ├── LEGAL.md                # Dépendances non permissives et leur statut
    ├── ARCHITECTURE.md
    ├── SECURITY.md
    ├── PRIVACY.md
    ├── BENCHMARKS.md
    ├── CONTRIBUTING.md
    ├── adr/                    # Architecture Decision Records
    ├── benchmarks/             # Machine de référence, scénarios, résultats
    └── user/                   # Documentation utilisateur
```

### 10.2 Couches (architecture hexagonale)

```text
UI
 ↓ commandes / événements
Application
 ↓ cas d'utilisation
Domaine (fx-domain)
 ↓ ports/traits
Infrastructure
 ├── Filesystem (fx-fs)
 ├── Windows Shell (fx-shell)
 ├── Index (fx-index)
 ├── SQLite (fx-storage)
 ├── Cloud (fx-cloud)
 ├── Réseau (fx-network)
 └── Codecs (fx-codec)
```

**Règles de dépendance :**

- `fx-domain`, `fx-fs` et `fx-ops` ne dépendent **jamais** de `fx-ui` ni du framework UI. Toute la logique métier est testable sans fenêtre.
- L'UI ne doit pas appeler directement les API Win32 ou COM.
- Seuls `fx-shell`, `fx-terminal`, `fx-git` et `fx-a11y` contiennent du code `unsafe` ou du COM brut. Tout `unsafe` est accompagné d'un commentaire `// SAFETY:` et d'un test ciblé.
- `fx-codec` et `fx-preview` exposent des traits ; les implémentations optionnelles sont derrière des feature flags.
- `fx-cli` réutilise `fx-domain`/`fx-fs`/`fx-index`, jamais `fx-ui`.

### 10.3 Modèle de threading

| Contexte | Rôle | Règle |
|---|---|---|
| **Thread UI** | Rendu et événements d'entrée | **Aucun I/O bloquant**, aucun calcul long |
| **Runtime `tokio`** | I/O async, réseau, cloud, orchestration | Communique avec l'UI via `invoke_from_event_loop` |
| **Pool `rayon`** | Parcours FS, hash, indexation initiale | Résultats streamés par lots, jamais d'attente bloquante côté UI |
| **Threads dédiés** | Lecture USN Journal, pompes ConPTY | Canaux `crossbeam` ou `tokio::sync::mpsc` |

Chaque tâche longue est **annulable** (`CancellationToken`) et **remontée** à l'UI avec un état (en attente, en cours, terminée, erreur).

### 10.4 Async et annulation

- Runtime Tokio pour les opérations asynchrones ; pools dédiés aux tâches bloquantes.
- `CancellationToken` pour navigation, recherche et previews.
- Identifiant de **génération** pour rejeter les résultats obsolètes (changement de dossier pendant un listing).
- Backpressure sur les canaux ; priorité aux tâches visibles.
- Aucun `.await` ou appel bloquant non maîtrisé sur le thread UI.

### 10.5 COM et interop Windows

- Initialisation COM explicite par thread ; respect STA/MTA selon l'API.
- Wrappers RAII ; limitation stricte du code `unsafe` ; chaque bloc documenté.
- Tests sur Windows réel ; gestion des objets Shell lents ou défaillants (timeouts, fallback — voir 13.8).

### 10.6 Positionnement Rust-first

Le code du projet est en Rust. L'interop Windows (COM, DLL système, Media Foundation, DirectWrite) est confinée dans les crates dédiés, documentée et testée. Aucune promesse d'absence totale de code natif externe (décision D-7).

### 10.7 Modèle de données principal

```rust
pub struct FileEntry {
    pub id: EntryId,
    pub path: PathBuf,            // OsString/UTF-16 pour chemins longs \\?\
    pub display_name: String,
    pub kind: EntryKind,          // fichier, dossier, lien, reparse, placeholder cloud…
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub attributes: FileAttributes,
    pub icon_key: IconKey,
    pub thumbnail_key: Option<ThumbnailKey>,
    pub cloud_state: Option<CloudState>,   // synchronisé, en cours, erreur, en ligne uniquement
    pub git_state: Option<GitState>,       // v1.5
    pub tags: Option<Tags>,                // v1.5
}

pub struct TabState {
    pub current_location: Location,
    pub back_history: Vec<NavigationState>,
    pub forward_history: Vec<NavigationState>,
    pub view: ViewSettings,
    pub selection: SelectionState,
    pub scroll_anchor: Option<EntryId>,
}

pub enum Location {
    Local(PathBuf),
    Unc(String),
    Wsl { distro: String, path: String },
    Search(SearchQuery),
    Archive { archive: PathBuf, inner_path: String },
    Cloud(CloudLocation),
    Virtual(VirtualLocation),
}
```

### 10.8 Cache multi-niveaux

| Niveau | Contenu | Politique |
|---|---|---|
| **L1 mémoire** | Icônes, miniatures, listings récents, aperçus | LRU borné, budget mémoire configurable |
| **L2 SQLite** | Récents, épinglés, tags, workspaces, configuration, métadonnées de listing | Persistant, migrations versionnées |
| **L3 disque** | Index `tantivy`, cache de miniatures, cache de hash | Dans `%LOCALAPPDATA%\FerrixExplorer\cache`, purgeable, reconstruction possible |

### 10.9 Stockage local

SQLite (`fx-storage`) pour : préférences, historique, récents, épingles, workspaces, tags, état des opérations, métadonnées du cache, migrations. Tantivy (`fx-index`) pour l'index de recherche.

**Exigences :** transactions ; migrations versionnées ; sauvegarde de sécurité avant migration destructive ; corruption détectable ; reconstruction possible des caches ; séparation stricte des données critiques et reconstructibles.

### 10.10 Emplacement des données

- Mode standard : `%APPDATA%\FerrixExplorer\` (configuration) et `%LOCALAPPDATA%\FerrixExplorer\` (cache, index).
- Mode portable : tout dans un dossier `data\` à côté de l'exécutable (marqueur spécifique requis). Aucune écriture dans le registre ni dans `%APPDATA%`.

---

## 11. Spike UI (Tranche 0)

Le framework UI **ne doit pas être choisi définitivement sans prototype**. Candidats : Slint ; WinUI via interop ; `windows-rs` avec Direct2D/DirectComposition ; tout autre framework Rust maintenu répondant aux critères.

Le spike compare, sur un prototype de fenêtre principale et de liste virtualisée de 100 000 éléments :

- virtualisation et latence ; consommation mémoire ; taille du binaire ;
- accessibilité UI Automation (annonce Narrator d'une liste virtualisée) ;
- drag-and-drop OLE et menus natifs ;
- rendu DPI, IME, multi-fenêtres, Snap Layouts, Mica ;
- **licence** (voir 9.1).

Décision documentée dans `docs/adr/0001-ui-framework.md` avant toute écriture d'UI de production.

---

## 12. Moteur Quick Look et compression

### 12.1 Principe directeur

Pour chaque type de contenu, Ferrix choisit **la bibliothèque la plus rapide disponible sous licence permissive**. Des moteurs plus rapides mais sous licence restrictive (LZTurbo, MuPDF, ffmpeg complet) sont ajoutés comme **backends optionnels**, installés et activés explicitement par l'utilisateur, **jamais par défaut**.

Il n'existe pas un décompresseur unique optimal pour tous les formats : chaque archive impose son format et son algorithme. Ferrix utilise donc un **registre de codecs** et sélectionne le backend le plus rapide et compatible pour le format rencontré.

### 12.2 Table des moteurs par type

| Type | Moteur par défaut (OSS, inclus) | Licence | Backend « performance » optionnel | Remarque |
|---|---|---|---|---|
| Images JPEG/PNG/WebP/AVIF | `zune-image` (Rust pur, SIMD) | MIT/Apache | `turbojpeg` (libjpeg-turbo) pour JPEG maximal | WIC en fallback ; décodage progressif |
| Images RAW | `rawloader` ou successeur (Rust pur) | MIT/Apache | `libraw` (LGPL), activable | Aperçu via vignette embarquée en priorité |
| Redimensionnement / miniatures | `fast_image_resize` (SIMD) | MIT/Apache | — | Miniatures en arrière-plan, priorisées par viewport |
| Archives ZIP / Deflate | `libdeflater` (bindings libdeflate) | MIT | **LZTurbo** (plugin externe, licence tierce) | Backend choisi selon préférence utilisateur |
| Archives 7z | `sevenz-rust` (Rust pur) | MIT/Apache | SDK 7-Zip natif en fallback | Lecture et création |
| Archives RAR | `unrar` (lecture seule) | ⚠️ Propriétaire | — | Feature `rar` (voir 9.3) |
| TAR/GZIP/XZ/BZIP2 | Crates Rust maintenues | MIT-family | Outil externe configuré | |
| Compression interne (cache, transferts) | `zstd` (mode long-range) | BSD | LZTurbo si installé | |
| PDF | `pdfium-render` (Pdfium, BSD) | BSD | — | MuPDF exclu (AGPL) |
| Vidéo (vignette et lecture) | Windows Media Foundation (natif) | Système | `ffmpeg` (plugin, LGPL/GPL selon build) | Zéro dépendance licence par défaut |
| Audio (waveform, métadonnées) | `symphonia` (ou Media Foundation) | MPL-2.0 | — | |
| Code source et texte | `tree-sitter` (incrémental) | MIT | `syntect` en fallback (grammaires Sublime) | Détection d'encodage obligatoire |
| Polices | DirectWrite (natif) | Système | — | Aperçu avec texte d'exemple |
| Office (docx, xlsx, pptx) | Shell Preview Handlers natifs | Système | Parsing custom en v2 | Métadonnées en dernier recours |
| Listing d'archive (aperçu) | Backend d'archive ci-dessus | — | — | Pas d'extraction |

Les bibliothèques exactes sont validées par un ADR portant sur : licence, sécurité, maintenance, performance, compatibilité ARM64, coût de distribution, historique de vulnérabilités.

### 12.3 Architecture pluggable (traits Rust)

```rust
/// Backend de (dé)compression. Implémentations : builtin, perf (opt-in), external (plugin).
pub trait CompressionBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn kind(&self) -> BackendKind;              // Builtin | OptionalPerf | External
    fn supports(&self, format: ArchiveFormat) -> bool;
    fn decompress(&self, input: &[u8], out: &mut Vec<u8>) -> Result<usize, CodecError>;
    fn compress(&self, input: &[u8], level: u8, out: &mut Vec<u8>) -> Result<usize, CodecError>;
}

/// Registre de codecs archives : probe, liste, extraction.
pub trait ArchiveBackend: Send + Sync {
    fn id(&self) -> &'static str;
    fn probe(&self, source: &ArchiveSource) -> ProbeResult;
    fn list(&self, source: &ArchiveSource) -> Result<Vec<ArchiveEntry>>;
    fn extract(
        &self,
        source: &ArchiveSource,
        destination: &Path,
        options: ExtractOptions,
        cancel: CancellationToken,
    ) -> Result<ExtractReport>;
}

/// Fournisseur d'aperçu Quick Look.
pub trait PreviewProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, input: &PreviewInput) -> SupportLevel;   // extensions, MIME, signature
    fn priority(&self) -> u8;
    fn preview(
        &self,
        input: PreviewInput,
        context: PreviewContext,            // budget temps/taille, niveau de confiance
        cancel: CancellationToken,
    ) -> PreviewFuture;
}
```

Chaque provider déclare : les extensions supportées ; la détection MIME ou signature ; sa priorité ; ses limites ; son niveau de confiance ; son besoin éventuel d'isolation ; sa licence et ses dépendances.

**Règle de sélection du backend** (dans `BackendRegistry` et `PreviewRegistry`) :

1. Préférence explicite de l'utilisateur (paramètres), si le backend est disponible.
2. Backend `OptionalPerf` ou `External` activé et détecté.
3. Backend `Builtin` (toujours disponible).

**Règle de repli** : si un provider échoue ou dépasse son budget, le suivant est tenté, puis le Shell Preview Handler natif, puis le Windows Imaging Component, puis l'icône système. L'échec n'est jamais silencieux : il est journalisé (`tracing`) et signalé de façon discrète dans l'aperçu.

### 12.4 Expérience Quick Look

- Activation par **barre d'espace** sur l'élément sélectionné ; fermeture par Échap ou nouvelle barre d'espace.
- Navigation pendant l'aperçu : flèches haut/bas (élément précédent/suivant), plein écran (F11 ou double-clic).
- Rendu **asynchrone, annulable et limité en mémoire** : changer d'élément annule le rendu en cours.
- État de chargement visible ; formats à risque parsés hors processus / isolés (voir 14.3).
- Cache L1 des aperçus récents.
- Types MVP : images (+ GIF animés), PDF, texte, Markdown, code source, JSON/XML/YAML/TOML, audio, vidéo, polices, archives (listing), métadonnées, Office via Shell Handlers.
- Types v1.5 : RAW, SVG (politique de sécurité stricte), modèles 3D courants, e-books, aperçus enrichis Office, hex viewer.
- Types v2 : navigation dans les archives comme dossier virtuel (PRV-10), panneau EXIF enrichi (PRV-09).

### 12.5 Sécurité des archives

Le moteur doit protéger contre : Zip Slip et traversée de chemin ; chemins absolus ; liens symboliques dangereux ; bombes de décompression ; tailles déclarées mensongères ; archives récursives ; consommation excessive de mémoire ; noms invalides sous Windows ; écrasement silencieux ; fichiers spéciaux.

Avant extraction, l'interface affiche : taille compressée ; taille estimée après extraction ; nombre de fichiers ; avertissement de ratio suspect ; espace disque nécessaire.

### 12.6 Politique LZTurbo

LZTurbo est considéré comme un **backend expérimental optionnel** (voir 9.2). S'il n'est pas compatible : jamais dans le build officiel, jamais téléchargé automatiquement, pris en charge par un adaptateur externe explicitement installé par l'utilisateur, statut documenté dans `docs/LEGAL.md`. La même politique s'applique à tout codec propriétaire ou à licence restrictive.

---

## 13. Exigences fonctionnelles détaillées

> Ce chapitre développe, exigence par exigence, le backlog identifié de l'**Annexe A** : chaque sous-section porte les IDs concernés (`NAV-01`, `OPS-03`…). En cas d'écart, ce chapitre fait foi ; l'Annexe A sert d'index de priorisation.

### 13.1 Navigation (NAV-01 → NAV-16)

L'application doit permettre :

- la navigation dans les fichiers et dossiers locaux, les partages UNC, les lecteurs réseau mappés et les emplacements WSL ;
- la saisie directe d'un chemin, y compris chemins longs (`\\?\`) et Unicode ;
- le retour au dossier précédent, le passage au dossier suivant, la remontée au parent, l'actualisation ;
- l'ouverture de plusieurs fenêtres et de plusieurs onglets ;
- le déplacement, le détachement et la restauration des onglets ;
- l'ouverture d'un dossier dans un nouvel onglet, une nouvelle fenêtre ou l'autre panneau ;
- le double-clic : ouvrir un dossier (navigation) ou un fichier (application associée via `ShellExecute`) ;
- le filtrage à la volée (liste filtrée en temps réel) ;
- le tri multi-critères et le groupement ;
- la virtualisation de la liste (seuls les éléments visibles sont créés).

**Breadcrumb (NAV-04)** — affiche chaque segment du chemin ; permet de cliquer sur un segment ; affiche les sous-dossiers d'un segment ; peut être remplacé temporairement par une barre d'adresse éditable (NAV-05, avec autocomplétion) ; accepte les chemins locaux, UNC, URI compatibles et chemins WSL.

**Historique (NAV-06)** — chaque panneau dispose de son propre historique : précédent (`Alt+←`, bouton latéral souris), suivant (`Alt+→`), dossier parent, position de scroll et sélection courante lorsque possible.

### 13.2 Modes d'affichage (NAV-02, NAV-03, NAV-09, NAV-10, UX-06, UX-07)

Modes requis : liste ; détails ; petites icônes ; icônes moyennes ; grandes icônes ; très grandes icônes ; mosaïque ; contenu.

Fonctions associées : zoom avec `Ctrl+molette` ; mémorisation du mode d'affichage par dossier ou type de dossier ; colonnes redimensionnables et réordonnables ; tri ascendant/descendant et multi-critères ; groupement (type, date, taille) ; affichage des fichiers cachés et système ; affichage des extensions ; cases de sélection en option ; densité compacte/normale/confortable.

La liste doit être virtualisée et ne créer que les composants visibles.

### 13.3 Sélection (OPS-11)

- clic simple ; double-clic ; `Ctrl+clic` ; `Shift+clic` ; `Ctrl+A` ;
- sélection rectangulaire ; sélection au clavier ; inversion de sélection ; sélection par motif ;
- conservation raisonnable de la sélection après actualisation ;
- annonce accessible du nombre d'éléments sélectionnés.

### 13.4 Opérations sur les fichiers (OPS-01 → OPS-24, ORG-01)

**Fonctions requises** — copier ; couper ; coller ; déplacer ; supprimer vers la Corbeille ; supprimer définitivement (`Maj+Suppr`) ; renommer (F2) ; créer un dossier ; créer un fichier ; créer un raccourci ; ouvrir ; ouvrir avec ; afficher les propriétés ; dupliquer ; envoyer vers ; compresser ; extraire ; calculer un checksum (MD5, SHA-256, BLAKE3) ; copier le chemin ; copier le nom ; ouvrir un terminal à cet emplacement ; nettoyer les dossiers vides.

**Centre d'activité (OPS-17)** — toutes les opérations longues apparaissent dans un centre d'activité : progression globale et par fichier ; vitesse instantanée et moyenne ; temps estimé ; fichier en cours ; pause ; reprise ; annulation ; nouvelle tentative ; affichage des erreurs ; réduction possible du panneau. Les opérations passent par `IFileOperation` lorsque c'est possible, avec boîte de progression native (OPS-08).

**Conflits (OPS-09)** — remplacer ; ignorer ; conserver les deux ; comparer les métadonnées ; appliquer la décision à tous les conflits similaires ; annuler l'opération.

**Undo/Redo (OPS-05)** — historise renommage, déplacement, création de dossier, mise à la Corbeille, tags, épingles. **L'interface ne doit pas prétendre qu'une opération est réversible lorsqu'elle ne l'est pas.**

**Intégrité (OPS-18)** — vérification hash source/destination après copie, optionnelle et rapportée.

### 13.5 Renommage par lot (OPS-03)

Prévisualisation avant application ; recherche/remplacement ; expressions régulières ; numérotation ; changement de casse ; préfixe/suffixe ; utilisation de la date ; utilisation de métadonnées EXIF ; nettoyage des caractères invalides ; détection des collisions ; annulation de toute l'opération lorsque possible.

### 13.6 Sidebar (SID-01 → SID-10)

**Sections attendues** — Accueil ; Épinglés ; Récents ; Téléchargements ; Documents ; Images ; Musique ; Vidéos ; Bureau ; Ce PC ; volumes locaux ; volumes amovibles ; lecteurs réseau ; Réseau ; WSL ; comptes cloud ; tags ; recherches sauvegardées ; workspaces.

**Fonctions** — épingler/désépingler ; réordonner par glisser-déposer ; renommer un favori **sans renommer le dossier réel** ; réduire/développer une section ; afficher l'espace utilisé (jauge) pour les volumes ; afficher l'état des emplacements indisponibles ; conserver la configuration entre les sessions. Les bibliothèques viennent des KnownFolders (SID-04), la détection amovible est un hotplug (SID-07), Dev Drive est détecté (SYS-01).

### 13.7 Onglets, fenêtres et dual-pane (NAV-13 → NAV-16, SID-08)

**Onglets** — nouvel onglet (`Ctrl+T`) ; fermer (`Ctrl+W`) ; rouvrir l'onglet fermé (`Ctrl+Maj+T`) ; dupliquer ; épingler ; réordonner ; détacher ; déplacer vers une autre fenêtre ; fermer les autres ; fermer à droite ; restaurer la session.

**Dual-pane** — deux panneaux indépendants ; un onglet ou groupe d'onglets par panneau ; copie et déplacement rapides vers l'autre panneau ; échange des panneaux ; synchronisation optionnelle de la navigation ; comparaison visuelle ; focus clairement identifiable ; raccourcis inspirés des gestionnaires Commander.

**Workspaces (SID-08)** — un workspace sauvegarde : les fenêtres ; les onglets ; les chemins ; la disposition dual-pane ; les tailles de panneaux ; le terminal associé ; les filtres et recherches ; le moniteur et la position de fenêtre, si encore disponibles.

### 13.8 Menu contextuel Windows (SYS-03, SYS-04)

Fonctions : menu contextuel sur un fichier, sur plusieurs fichiers et sur le fond de dossier ; verbes Shell ; sous-menus ; extensions tierces ; commandes personnalisées Ferrix ; support clavier de la touche Menu et de `Shift+F10`.

**Isolation et stabilité** — les extensions Shell tierces peuvent être lentes ou instables. L'architecture prévoit : mesure du temps d'ouverture ; timeout ; journalisation technique ; liste de blocage locale ; désactivation d'une extension ; processus auxiliaire si l'isolation hors processus est réalisable ; fallback vers un menu Ferrix minimal.

La parité exacte avec les menus de Windows 10 et Windows 11 **n'est pas garantie** (Microsoft n'expose pas tous les comportements de façon uniforme aux applications tierces) — mais le clic droit doit afficher les mêmes entrées qu'Explorer pour les extensions courantes (7-Zip, Git…).

### 13.9 Drag-and-drop (OPS-06, OPS-07)

Doit fonctionner : entre dossiers Ferrix ; entre panneaux ; entre fenêtres ; depuis Ferrix vers l'Explorateur Windows et inversement ; vers et depuis les applications compatibles OLE ; vers la sidebar ; vers les onglets ; avec les modificateurs copie/déplacement/raccourci.

**Une alternative clavier doit exister pour chaque action essentielle.**

### 13.10 Recherche et indexation (SRC-01 → SRC-07)

**Recherche dans le dossier courant** — sans index obligatoire ; filtre instantané par nom ; filtre sur extension, taille et date ; recherche récursive optionnelle ; annulation immédiate.

**Recherche globale** — l'index contient au minimum : nom ; chemin ; extension ; taille ; dates ; identifiant de volume ; identifiant de fichier lorsque disponible ; attributs ; type de fichier.

**Syntaxe de recherche** — `name:rapport` ; `ext:pdf` ; `size:>100MB` ; `modified:this-week` ; `path:C:\Projects` ; `kind:image` ; `is:hidden` ; `is:directory`.

**Indexation** — 1) lecture de l'USN Journal sur les volumes NTFS lorsque possible ; 2) parcours classique (`jwalk`) en fallback sans privilèges ; 3) écoute des changements USN et filesystem ; 4) réconciliation périodique ; 5) mise à jour transactionnelle de l'index (`tantivy`) ; 6) exclusion configurable de chemins ; 7) limitation des ressources en arrière-plan ; 8) progression et contrôle (pause/reprise) visibles.

L'indexation **ne doit pas exécuter une élévation permanente**. Sans privilèges, l'application fonctionne en mode dégradé avec un message clair (SRC-05).

**Recherche de contenu (SRC-06)** — optionnelle et désactivable : texte brut ; code source ; PDF lorsque l'extraction est disponible ; documents Office via extracteur autorisé ; taille maximale configurable ; exclusions par type ou emplacement ; aucun envoi vers un service distant.

### 13.11 Palette de commandes (PWR-01)

Raccourci par défaut : `Ctrl+Maj+P` (`Ctrl+Shift+P`). Permet de : rechercher une commande ; ouvrir un dossier récent ; changer de vue ; basculer le thème ; ouvrir un terminal ; calculer un checksum ; lancer un renommage par lot ; gérer les onglets ; exécuter une action de plugin ; afficher le raccourci associé ; tout exécuter sans souris.

**La liste des commandes provient d'un registre central partagé avec les menus et les raccourcis.**

### 13.12 Terminal intégré (PWR-02)

ConPTY lorsque disponible. Fonctions : PowerShell ; CMD ; Windows Terminal externe ; profils WSL ; shell personnalisé ; ouverture dans le dossier courant ; panneau inférieur détachable ; plusieurs sessions ; redimensionnement ; copier/coller ; couleurs ANSI ; liens cliquables ; recherche ; raccourcis configurables.

Le terminal **ne doit pas transmettre automatiquement de commandes à un shell élevé**.

### 13.13 CLI compagnon (PWR-03, EXT-04)

Binaire `fx` :

```powershell
fx .
fx open C:\Projects
fx search "ext:rs path:C:\Projects"
fx workspace open development
fx checksum SHA256 .\archive.zip
fx reveal .\target\release\app.exe
```

Le CLI doit : communiquer avec une instance existante lorsque possible ; démarrer Ferrix si nécessaire ; retourner des codes d'erreur documentés ; fournir une sortie lisible et une sortie JSON pour l'automatisation.

### 13.14 Git (PWR-06)

Fonctions v1.5 : détection des dépôts ; badges ajouté, modifié, supprimé, ignoré et non suivi ; branche courante ; invalidation du cache après modification (en < 1 s) ; désactivation globale ou par dossier ; limitation du coût sur les monorepos ; respect de `.gitignore`.

**Ferrix ne doit pas devenir un client Git complet dans le MVP.**

### 13.15 WSL (PWR-04)

Découverte des distributions ; navigation via les chemins WSL exposés par Windows (`\\wsl$\`) ; ouverture dans un terminal WSL ; copie de chemin Windows et de chemin Linux ; avertissement pour les opérations potentiellement lentes ; gestion correcte de la casse ; affichage des liens symboliques ; **pas d'hypothèse NTFS sur les fichiers WSL**.

### 13.16 Cloud (CLD-01 → CLD-06)

**Principes** — Ferrix ne réimplémente pas la synchronisation OneDrive, Google Drive ou Dropbox lorsqu'un client officiel est installé. Priorités : 1) détecter les racines locales synchronisées ; 2) afficher les placeholders ; 3) afficher les états de disponibilité ; 4) utiliser les comportements du client officiel ; 5) demander le téléchargement à la demande via les API documentées ; 6) proposer une intégration distante optionnelle ultérieure.

**Providers** — OneDrive ; OneDrive Entreprise ; Google Drive pour ordinateur ; Dropbox ; providers compatibles Cloud Files API (CfApi) ; autres providers via plugins à terme.

**États** — disponible localement ; toujours disponible ; en ligne uniquement ; synchronisation en cours ; erreur ; conflit ; provider déconnecté.

**APIs distantes (optionnelles et séparées)** — OAuth2 avec navigateur système ; PKCE ; stockage des tokens dans le Windows Credential Manager ; permissions minimales ; révocation de session ; aucun secret client embarqué lorsque possible ; journalisation sans token.

### 13.17 Réseau (NET-01, NET-02)

**MVP** — chemins UNC ; lecteurs réseau mappés ; reconnexion automatique ; gestion des timeouts ; affichage d'un état hors ligne ; annulation des requêtes lentes.

**v1.5/v2** — découverte SMB (voisinage réseau) ; SFTP ; FTP/FTPS ; WebDAV ; profils de connexion ; secrets dans le Credential Manager ; favoris réseau ; limitation du cache local ; fichiers hors ligne (cache réseau).

**Les emplacements réseau ne doivent jamais bloquer l'ouverture de l'application.**

### 13.18 Organisation et analyse (ORG-01 → ORG-05, OPS-19, OPS-20, PRV-10)

**Tags (ORG-02)** — tags colorés et multiples ; stockage par identifiant stable lorsque possible ; fallback par chemin ; recherche par tag ; export/import. Les tags ne modifient pas silencieusement le fichier ou les ADS sans option explicite.

**Recherches sauvegardées** — requête ; emplacements ; filtres ; tri ; mode d'affichage.

**Analyse d'espace disque (ORG-03)** — treemap ; classement par taille ; regroupement par extension ; scan annulable ; exclusion des liens et reparse points ; distinction claire tailles logiques/physiques ; export du rapport.

**Détection des doublons (OPS-19)** — 1) regroupement par taille ; 2) hash partiel ; 3) hash complet ; 4) comparaison finale optionnelle. Fonctions : prévisualisation ; sélection assistée ; liens physiques lorsque pertinent ; déplacement vers la Corbeille ; **aucune suppression automatique par défaut**.

**Montage ISO/VHD (OPS-20)** — via les API Windows lorsque possible (passthrough = effort faible).

**Archives comme dossiers virtuels (PRV-10)** — navigation dans les archives sans extraction.

### 13.19 Propriétés et métadonnées (OPS-14, PRV-09)

Afficher : nom ; type ; chemin ; taille logique ; taille sur disque ; dates ; attributs ; propriétaire ; permissions ; checksum ; flux alternatifs NTFS ; reparse point ; cible de lien ; informations EXIF ; informations média ; signature numérique ; version du binaire ; état cloud.

Les opérations sensibles sur ACL sont reportées en v2 (SYS-08).

### 13.20 Interface et personnalisation (UX-01 → UX-10)

**Thèmes** — clair ; sombre ; suivre Windows (changement en < 1 s) ; contraste élevé ; couleurs d'accentuation ; réduction des animations ; Mica/Acrylic lorsque disponible ; fallback opaque sur Windows 10 ou matériel incompatible.

**Personnalisation** — densité compacte/normale/confortable ; colonnes ; barre d'outils ; taille des icônes ; sidebar ; raccourcis ; application terminal ; vue par type de dossier ; ouverture simple ou double-clic ; comportement au démarrage ; packs d'icônes (Community).

**Multi-écrans et DPI (A11Y-05, UX-05)** — restauration sur le bon écran ; gestion du changement de DPI et des DPI mixtes ; mémorisation raisonnable de la position ; retour automatique sur un écran disponible.

**Système (UX-04, SYS-05, SYS-06, SYS-07, SYS-09)** — mode portable (marqueur spécifique, données à côté de l'exécutable, ni registre ni `%APPDATA%`) ; récupération de session après plantage ; élévation UAC ponctuelle sans relancer l'application ; option « explorateur par défaut » si techniquement propre ; Jump Lists ; télémétrie opt-in ; Dev Drive (SYS-01) ; indicateurs de fichiers compressés/chiffrés (SYS-02).

### 13.21 Accessibilité (A11Y-01 → A11Y-07)

**Exigences obligatoires** — UI Automation ; Narrator ; navigation complète au clavier ; ordre de tabulation cohérent ; focus visible ; noms/rôles/états accessibles ; annonce des changements de dossier ; annonce des opérations et erreurs ; contraste élevé ; zoom ; respect des préférences de réduction d'animation ; actions alternatives au drag-and-drop ; indicateurs pas uniquement fondés sur la couleur ; vérification avec Accessibility Insights for Windows.

**Critères de validation** — toutes les fonctionnalités MVP utilisables sans souris ; Narrator annonce le nom, le type et l'état des éléments ; les dialogues ne piègent pas le focus ; les listes virtualisées restent accessibles ; les tests d'accessibilité sont intégrés à **chaque** tranche UI.

### 13.22 Raccourcis clavier (PWR-05)

| Action | Raccourci | Action | Raccourci |
|---|---|---|---|
| Nouvel onglet | `Ctrl+T` | Renommer | `F2` |
| Fermer l'onglet | `Ctrl+W` | Actualiser | `F5` |
| Rouvrir l'onglet | `Ctrl+Maj+T` | Supprimer | `Suppr` |
| Changer d'onglet | `Ctrl+Tab` | Supprimer définitivement | `Maj+Suppr` |
| Nouvelle fenêtre | `Ctrl+N` | Propriétés | `Alt+Entrée` |
| Barre d'adresse | `Ctrl+L` ou `Alt+D` | Précédent | `Alt+←` |
| Recherche | `Ctrl+F` | Suivant | `Alt+→` |
| Palette de commandes | `Ctrl+Maj+P` | Parent | `Alt+↑` |
| Terminal | `` Ctrl+` `` | Tout sélectionner | `Ctrl+A` |
| Aperçu rapide | `Espace` | Copier / Couper / Coller | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` |
| Menu contextuel | `Maj+F10` | Annuler | `Ctrl+Z` |

Parité Explorer complète (tous les raccourcis standards fonctionnent) + extensions. Les raccourcis sont **remappables** avec détection des conflits. Un mode Vim optionnel pourra être ajouté par la communauté (PWR-07).

---

## 14. Exigences non fonctionnelles

### 14.1 Performance et benchmarks

Cibles chiffrées : section 5.2. Règles :

- Ne jamais bloquer le thread UI ; virtualiser toute liste longue ; miniatures chargées uniquement si visibles ou bientôt visibles ; annulation des tâches obsolètes ; caches LRU bornés ; mesurer avant d'optimiser.
- Toute régression supérieure à **10 %** sur un scénario de benchmark bloque la fusion (CI).
- Machine de référence documentée dans `docs/benchmarks/REFERENCE.md` (Tranche 0).

**Scénarios de benchmark** : 1 000 fichiers ; 10 000 fichiers ; 100 000 fichiers ; 1 million d'entrées synthétiques ; miniatures d'images ; dossier réseau lent ; lecteur déconnecté ; archive volumineuse ; recherche globale ; copie de petits fichiers ; copie d'un fichier très volumineux.

**Comparaison** : Ferrix vs `explorer.exe` vs autres gestionnaires populaires, dans les mêmes conditions.

**Rapport** : machine ; version Windows ; stockage ; état du cache ; médiane ; p95 ; nombre de répétitions. Publié dans `docs/benchmarks/` à chaque release.

### 14.2 Fiabilité

- Aucune opération destructive sans confirmation ou passage par la Corbeille (sauf suppression définitive explicitement demandée).
- Jamais de confirmation de succès avant confirmation par le système ; opérations interrompues (coupure, débranchement USB) laissent des fichiers cohérents ; opérations incomplètes signalées.
- Pas de crash sur chemin invalide, nom illégal, permissions refusées, disque plein, fichier verrouillé : erreurs journalisées et présentées à l'utilisateur.
- Opérations longues annulables ; reprise et reporting d'erreurs détaillés.
- Récupération de session après plantage (SYS-05) ; index réparable et reconstruisible.

### 14.3 Sécurité

**Modèle de menace** — fichiers malformés ; miniatures hostiles ; archives malveillantes ; extensions Shell instables ; chemins réseau compromis ; plugins non fiables ; tokens cloud ; traversée de répertoires ; reparse points ; élévation de privilèges ; DLL hijacking ; commandes terminal injectées ; bombes de décompression (12.5).

**Mesures** —

- parsing hors processus pour les formats à risque lorsque possible ; Job Objects et limites de ressources ;
- chemins canoniques vérifiés ; politique stricte sur les reparse points ;
- jetons OAuth dans le Windows Credential Manager, jamais en clair dans la config ;
- plugins WASM sans accès par défaut : permissions déclarées et demandées, aucune permission sensible implicite ; protocole IPC versionné ;
- signature des releases ; pas d'auto-update non signé ;
- `cargo audit` et `cargo deny` en CI ; revue de sécurité des dépendances ; SBOM ;
- aucun contenu de fichier, token ou chemin sensible dans les logs.

**Sécurité des plugins (v2)** — chaque plugin déclare ses capacités : lire une sélection ; lire un fichier ; écrire un fichier ; utiliser le réseau ; ajouter une commande ; ajouter un provider de preview ; accéder au presse-papiers. Un plugin sans permission réseau ne peut pas en utiliser.

### 14.4 Confidentialité et télémétrie

- Aucune télémétrie obligatoire ; aucun compte requis ; aucun chemin de fichier envoyé par défaut.
- Crash reporting **explicitement opt-in** ; affichage exact des données collectées ; possibilité d'exporter le rapport avant envoi ; possibilité de tout désactiver.
- Recherches et indexation **locales**.
- Documentation dans `docs/PRIVACY.md`.

### 14.5 Logs et diagnostics

Niveaux `error`/`warn`/`info`/`debug`/`trace` via `tracing`. Exigences : rotation et taille maximale (fichier rotatif dans `%LOCALAPPDATA%`) ; redaction des secrets ; chemins anonymisés dans les rapports exportés ; bundle de diagnostic volontaire ; journal séparé pour indexation et opérations ; aucune donnée de fichier lue dans les logs.

### 14.6 Compatibilité

**Systèmes (par ordre de priorité)** : 1) Windows 11 x64 ; 2) Windows 10 22H2 x64 ; 3) Windows 11 ARM64 ; 4) Windows 10 ARM64 si raisonnablement supportable. Fonctionnalités Win11 (Mica, Snap Layouts, Dev Drive) avec repli propre sur Win10.

**Systèmes de fichiers** : NTFS ; ReFS ; exFAT ; FAT32 ; partages SMB ; systèmes exposés par WSL ; fichiers cloud placeholders.

Toutes les fonctionnalités ne sont pas disponibles sur tous les systèmes de fichiers : **l'UI affiche les limites sans échouer silencieusement.**

### 14.7 Internationalisation

Textes externalisés dès la Tranche 0 (fichiers de ressources) ; FR et EN livrés en 1.0 ; tableaux et messages cohérents ; la langue du code et des commentaires reste l'anglais.

### 14.8 Maintenabilité

- Couverture de tests élevée sur `fx-domain`, `fx-fs`, `fx-ops`, `fx-index`, `fx-codec`, `fx-preview`.
- Rustdoc sur toutes les API publiques ; README dans chaque crate.
- ADR pour toute décision structurante ; `docs/LEGAL.md` mis à jour à chaque dépendance.
- Documentation utilisateur dans `docs/user/`.

---

## 15. Tests, validation et CI

### 15.1 Tests unitaires

Normalisation des chemins ; conflits de noms ; renommage par lot ; syntaxe de recherche ; calcul des opérations ; règles de sélection ; migrations SQLite ; choix de codecs ; validation des archives ; permissions de plugins.

### 15.2 Tests d'intégration

Filesystem temporaire ; copie/déplacement ; Corbeille ; liens et reparse points ; chemins longs ; Unicode ; fichiers verrouillés ; volumes amovibles ; SQLite ; index ; archives.

### 15.3 Tests Windows réels

Menus contextuels ; drag-and-drop ; propriétés ; COM ; Cloud Files API ; ConPTY ; Narrator ; multi-DPI ; thème sombre ; contraste élevé ; Snap Layouts. Le COM est difficile à simuler : **tests manuels documentés** pour `fx-shell`.

### 15.4 Tests de robustesse

Fuzzing des parsers ; archives malformées ; chemins invalides ; coupure réseau ; lecteur retiré ; processus Shell bloqué ; disque plein ; permissions refusées ; crash pendant migration ; crash pendant copie.

### 15.5 CI

**À chaque pull request** : `cargo fmt --check` ; `cargo clippy --all-targets --all-features -- -D warnings` ; tests unitaires ; tests d'intégration compatibles CI ; `cargo audit` ; vérification des licences (`cargo deny`) ; build Windows x64 ; benchmarks de non-régression si un chemin chaud a été touché.

**Périodiquement** : ARM64 ; tests E2E ; benchmarks complets ; fuzzing ; packaging.

---

## 16. Roadmap en tranches

> Chaque tranche est une version livrable et testable. Une tranche ne démarre qu'une fois les critères d'acceptation de la précédente validés. **L'accessibilité (UIA) et la performance sont transverses** : vérifiées à chaque tranche, pas seulement à la fin (décision D-3).

### 16.1 Vue d'ensemble

| # | Tranche | Contenu clé | Priorité |
|---|---|---|---|
| 0 | Fondations et validation technique | Workspace multi-crates, CI, logging, squelette `fx-a11y`, spikes UI/COM/UIA, décision UI | MVP |
| 1 | Navigation de base | Listing asynchrone, vues, breadcrumb, historique, filtrage, chemins longs, virtualisation | MVP |
| 2 | Opérations fichiers | Copier/couper/coller, renommer, supprimer, créer, sélection, undo/redo, checksums, centre d'activité | MVP |
| 3 | Shell natif | `IContextMenu`, `IFileOperation`, propriétés, « Ouvrir avec », OLE drag-and-drop, miniatures | MVP |
| 4 | Sidebar et lecteurs | Épinglés, accès rapide, récents, bibliothèques, Ce PC, hotplug, WSL, Dev Drive, treeview | MVP |
| 5 | Onglets, fenêtres, dual-pane | Onglets, multi-fenêtres, split view, mode Commander, workspaces, restauration de session | MVP |
| 6 | Recherche globale | USN/MFT, tantivy, mise à jour en direct, syntaxe, filtres, repli, plein texte optionnel | MVP |
| 7 | Power-user | Palette, terminal ConPTY, CLI `fx`, WSL, renommage regex, raccourcis et remappage | MVP |
| 8 | Quick Look et compression | `fx-preview`, `fx-codec`, aperçus complets, archives (ZIP, 7z, TAR…), sécurités d'extraction | MVP |
| 9 | Personnalisation Win11 | Thèmes, Mica, Snap Layouts, zoom, vues, colonnes, densité, mode portable, DPI, multi-écrans | MVP |
| 10 | Accessibilité et internationalisation | UIA complet, contraste élevé, audit Narrator/NVDA, FR/EN | MVP |
| 11 | Cloud | OneDrive (CfApi), Google Drive, Dropbox, quotas, OAuth2 | v1.5 |
| 12 | Outils avancés | Doublons, treemap, file d'attente, intégrité, tags, EXIF, archives virtuelles, ISO/VHD | v1.5 |
| 13 | Dev et système | Badges Git, Jump Lists, récupération de session, UAC, défaut, télémétrie opt-in | v1.5 |
| 14 | Réseau | SMB (voisinage réseau), FTP/SFTP/WebDAV | v1.5 / v2 |
| 15 | Performance | Benchmarks 1 M fichiers, profiling, cache GPU, cold start, fuites mémoire | Transverse |
| 16 | Extensibilité | Plugins WASM, Rhai, associations, diff, smart folders, suppression sécurisée, versions, ACL | v2 |
| 17 | Packaging | MSI/MSIX, signature, mises à jour signées, SBOM, version portable | MVP (release) |
| 18 | Release 1.0 | Documentation utilisateur, guide contributeur, bêta publique, annonce | MVP (release) |

### 16.2 Jalons de livraison

- **Prototype technique** — tranches 0 à 1 : prouver la faisabilité de l'UI, de l'accessibilité et de la navigation performante.
- **Alpha** — tranches 2 à 5 : explorateur quotidien local (opérations, sidebar, onglets, Shell).
- **Bêta** — tranches 6 à 10 : recherche, productivité, Quick Look, archives, personnalisation, accessibilité.
- **Version 1.0** — tranches MVP stabilisées (11, 17, 18), packaging et documentation. Le cloud avancé, les outils spécialisés et les plugins peuvent être livrés progressivement si leur stabilité retarde excessivement la 1.0.

### 16.3 Détail des tranches

#### Tranche 0 — Fondations et validation technique

**Objectif** : workspace fonctionnel, fenêtre vide, CI verte, décision UI prise, hauts risques techniques éprouvés.

**Contenu** — workspace multi-crates (10.1) ; `rust-toolchain.toml` ; `CLAUDE.md` ; prototypes Slint **et** iced (liste de 100 000 lignes) ; prototypes UI Automation, COM, drag-and-drop et `IContextMenu` ; fenêtre principale (titre, icône, taille mémorisée) ; squelette `fx-a11y` (élément racine : nom + rôle UIA) ; logging `tracing` rotatif ; CI GitHub Actions (build, tests, `clippy -D warnings`, `fmt`, `cargo deny`, `cargo audit`) ; `docs/LEGAL.md`, `docs/benchmarks/REFERENCE.md` et `docs/adr/0001-ui-framework.md` ; textes externalisés dès maintenant.

**Critères d'acceptation** — `cargo run` ouvre une fenêtre sur Windows 10 et 11 ; build reproductible ; CI verte ; Narrator annonce le nom de la fenêtre ; liste de 100 000 lignes restant interactive ; décision UI documentée dans l'ADR.

#### Tranche 1 — Navigation de base

**Objectif** : parcourir le système de fichiers en lecture seule, de façon fluide.

**Contenu** — `fx-fs`/`fx-domain` : `FileEntry`, abstraction `Location`, listing async (`jwalk`) streamé par lots, annulation, surveillance du dossier (`notify`) ; vues liste et détails, tri simple, virtualisation ; double-clic (navigation / `ShellExecute`) ; barre d'adresse éditable, breadcrumb, historique back/forward (y compris boutons souris) ; filtrage à la volée ; chemins longs `\\?\` et Unicode ; icônes système (`SHGetFileInfo`) ; navigation clavier de base et rôles UIA sur la liste et les colonnes.

**Critères d'acceptation** — navigation complète dans `C:\` et Known Folders, retour/avant fonctionnels ; aucun blocage UI pendant le listing ; dossier de 50 000 fichiers : premiers éléments en < 100 ms, scroll 60 FPS ; chemin > 260 caractères lisible et navigable ; Unicode correct ; Narrator annonce les éléments et les colonnes ; tests de navigation verts.

#### Tranche 2 — Opérations fichiers

**Objectif** : CRUD complet avec sécurité (Corbeille, undo).

**Contenu** — copier/couper/coller (presse-papiers Windows, `CF_HDROP`, `Preferred DropEffect`) ; renommer (F2), créer dossier et fichier ; supprimer vers la Corbeille, suppression définitive explicite (`Maj+Suppr`) ; sélection multiple et rectangulaire ; glisser-déposer interne ; boîte de conflit ; **centre d'activité** (progression, vitesse, erreurs) ; undo/redo (restauration depuis la Corbeille) ; checksums (MD5, SHA-256, BLAKE3) ; nettoyeur de dossiers vides ; barre de statut enrichie.

**Critères d'acceptation** — toutes les opérations fonctionnent avec retour visuel de progression ; fichiers supprimés retrouvés dans la Corbeille Windows ; `Ctrl+Z` annule la dernière opération réversible ; les conflits proposent les quatre choix ; **aucune fausse confirmation de succès**.

#### Tranche 3 — Shell natif

**Objectif** : comportements identiques à Explorer là où c'est attendu.

**Contenu** — menu contextuel natif `IContextMenu` (fusion des entrées, extensions tierces) + menu Ferrix ; `IFileOperation` avec boîte de progression native ; propriétés natives ; « Ouvrir avec » ; miniatures `IThumbnailProvider` avec cache disque ; drag-and-drop OLE dans les deux sens ; mesures de temps d'ouverture et fallback ; menu utilisable au clavier (`Maj+F10`).

**Critères d'acceptation** — le clic droit affiche les mêmes entrées qu'Explorer (7-Zip, Git si installés) ; un fichier glissé vers le Bureau ou Explorer est copié correctement, et inversement ; un fichier de 2 Go affiche la boîte de progression native ; le menu est utilisable au clavier ; un crash d'extension ne corrompt pas la session autant que techniquement possible.

#### Tranche 4 — Sidebar et lecteurs

**Objectif** : navigation par la barre latérale complète.

**Contenu** — épinglés (glisser-déposer, persistance SQLite) ; accès rapide, récents, bibliothèques (KnownFolders) ; Ce PC avec jauge d'espace ; lecteurs réseau mappés et UNC ; hotplug USB ; treeview ; Dev Drive ; WSL ; emplacements indisponibles affichés sans blocage.

**Critères d'acceptation** — épinglés persistants et réordonnables ; une clé USB apparaît sans redémarrage et disparaît proprement ; jauges d'espace conformes à Explorer à 1 % près ; WSL accessible lorsqu'installé ; aucun blocage UI sur emplacement indisponible.

#### Tranche 5 — Onglets, fenêtres, dual-pane

**Objectif** : productivité multi-contexte.

**Contenu** — onglets (`Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`, réordonnancement, détachement) ; multi-fenêtres indépendantes ; split view et mode dual-pane Commander (copie vers le panneau opposé, navigation synchronisée optionnelle) ; workspaces sauvegardés/rechargeables ; restauration des onglets à l'ouverture (la récupération après plantage complète est en Tranche 13).

**Critères d'acceptation** — gestion d'onglets comparable à un navigateur (réordonnancement fluide, détachement sans perte d'état) ; un workspace nommé se restaure à l'identique ; panneaux indépendants ; focus clavier non ambigu.

#### Tranche 6 — Recherche globale

**Objectif** : recherche instantanée façon Everything.

**Contenu** — lecture MFT et USN Journal sur tous les volumes ; index `tantivy` (nom, chemin, taille, dates, attributs, type) ; mise à jour en direct ; réconciliation périodique ; exclusions configurables ; repli `jwalk` sans privilèges (message clair) ; UI de résultats avec surlignage ; syntaxe de recherche (13.10) ; filtres avancés ; plein texte optionnel (désactivé par défaut) ; progression et contrôle de l'indexation.

**Critères d'acceptation** — indexation initiale d'un volume de 1 M de fichiers en < 60 s ; recherche par nom < 50 ms (p95) après indexation ; création/suppression visible en < 2 s ; mode sans privilèges fonctionnel avec message clair ; UI toujours interactive ; index réparable ; **aucune élévation permanente exigée**.

#### Tranche 7 — Power-user

**Objectif** : productivité clavier et outils développeur.

**Contenu** — palette de commandes (`Ctrl+Maj+P`) branchée sur un registre central partagé menus/raccourcis ; terminal ConPTY dans le dossier courant (PowerShell, cmd, WSL) ; CLI `fx` (texte et JSON) ; support WSL (`\\wsl$\`) ; renommage par lot regex avec aperçu ; raccourcis clavier : parité Explorer + remappage.

**Critères d'acceptation** — tous les raccourcis standards d'Explorer fonctionnent ; terminal ouvert dans le bon dossier en < 500 ms ; `fx open .` ouvre une fenêtre sur le dossier courant ; renommage prévisualisé et annulable ; toutes les actions principales accessibles par la palette.

#### Tranche 8 — Quick Look et compression

**Objectif** : aperçu instantané et archives sûrs.

**Contenu** — `fx-preview` : providers images, RAW, PDF, vidéo, audio, texte/code, archives (listing), polices, Office (Shell) ; aperçu par barre d'espace, navigation clavier, plein écran, rendu asynchrone annulable, cache L1, isolation des formats à risque ; `fx-codec` : registre de backends (ZIP/libdeflater, 7z/sevenz-rust, TAR/GZIP/XZ/BZIP2, zstd interne), backends optionnels via le registre ; création et extraction ; lecture RAR via feature `rar` ; extraction sécurisée (12.5) ; progression, mots de passe ; benchmarks des backends.

**Critères d'acceptation** — image standard aperçue en < 100 ms ; première page PDF en < 300 ms ; changer d'élément annule le rendu sans fuite de ressources ; un fichier malformé ne fait pas tomber l'app principale ; limites mémoire testées ; l'application compile et fonctionne **sans** aucun backend optionnel ; LZTurbo n'est ni inclus, ni téléchargé, ni référencé par défaut ; **protégé contre Zip Slip et bombes de décompression** ; aucune dépendance restrictive dans le build par défaut ; résultats de benchmarks publiés.

#### Tranche 9 — Personnalisation Windows 11

**Objectif** : look natif et réglages visuels.

**Contenu** — thèmes clair/sombre synchronisés Windows + surcharge ; Mica avec repli Win10 ; Snap Layouts ; vues icônes/mosaïque/contenu ; zoom `Ctrl+molette` ; densité ; colonnes et barre d'outils personnalisables ; indicateurs compressé/chiffré ; mode portable ; géométrie par écran ; DPI et taille de texte système (DPI mixtes).

**Critères d'acceptation** — thème suit le mode sombre Windows en < 1 s et sans redémarrage ; Mica visible sur Win11, repli propre sur Win10 ; une copie en mode portable ne modifie ni le registre ni `%APPDATA%` ; la fenêtre retrouve sa position sur le bon écran après débranchement d'un moniteur ; DPI mixtes fonctionnels ; réglages persistants.

#### Tranche 10 — Accessibilité et internationalisation

**Objectif** : utilisable sans souris et avec lecteur d'écran.

**Contenu** — UI Automation complet (listes virtualisées, sidebar, onglets, palette, dialogues, aperçu) ; annonces (dossier, résultats, opérations, erreurs) ; contraste élevé ; audit clavier sans piège de focus ; traduction FR/EN ; audit Narrator + NVDA + Accessibility Insights, documenté.

**Critères d'acceptation** — un parcours complet (naviguer, copier, renommer, rechercher, ouvrir l'aperçu) est réalisable au clavier et à l'oreille avec NVDA ; aucune fonction MVP uniquement à la souris ; aucun contrôle sans nom accessible (vérification automatisée UIA) ; aucun défaut critique dans Accessibility Insights ; mode contraste élevé respecte les couleurs système.

#### Tranche 11 — Cloud (v1.5)

**Contenu** — trait `CloudProvider` (list, status, quota, sync_state) ; OneDrive (racines, placeholders, badges CfApi) ; Google Drive et Dropbox (détection client local, repli API) ; section « Comptes cloud » dans la sidebar, quotas ; OAuth2 + PKCE via navigateur ; hydratation/libération à la demande.

**Critères d'acceptation** — badges cohérents avec le client officiel ; un fichier « en ligne uniquement » téléchargeable à la demande ; jetons OAuth dans le Credential Manager ; aucune réimplémentation du moteur de synchronisation.

#### Tranche 12 — Outils avancés (v1.5)

**Contenu** — file d'attente d'opérations (pause, reprise, réordonnancement, annulation) ; vérification d'intégrité post-copie ; détecteur de doublons par étapes ; treemap d'usage disque ; tags colorés ; panneau EXIF ; archives comme dossiers virtuels ; montage ISO/VHD/VHDX.

**Critères d'acceptation** — le détecteur de doublons ne lit jamais un fichier entier si la taille ou le hash partiel suffit ; **aucune suppression automatique** ; la treemap d'un disque de 500 Go se génère sans bloquer l'interface ; scans annulables ; rapports exportables.

#### Tranche 13 — Dev et système (v1.5)

**Contenu** — badges Git (modifié, ajouté, ignoré…) avec cache et invalidation ; Jump Lists ; récupération de session après plantage ; élévation UAC ponctuelle ; « explorateur par défaut » si techniquement propre ; télémétrie opt-in et rapports de crash anonymes ; WSL amélioré (copie chemins Linux/Windows) ; profils de terminal.

**Critères d'acceptation** — badges Git à jour en < 1 s après `git add` ; après un plantage, la session est restaurée au redémarrage ; dépôts volumineux utilisables ; fonctions désactivables.

#### Tranche 14 — Réseau (v1.5 / v2)

**Contenu** — découverte SMB (voisinage réseau) ; clients FTP, FTPS, SFTP et WebDAV navigables comme un dossier local ; profils de connexion ; secrets dans le Credential Manager ; favoris réseau ; timeouts et reprise.

**Critères d'acceptation** — un partage SMB découvert apparaît dans la sidebar ; une connexion SFTP permet copier, renommer et supprimer ; **aucune attente réseau sur le thread UI** ; secrets protégés ; erreurs réseau compréhensibles.

#### Tranche 15 — Performance (transverse)

**Contenu** — benchmarks automatisés (1k, 10k, 100k, 1M) avec comparaison `explorer.exe` ; profiling (`tracing`, Windows Performance Analyzer) ; cache GPU des icônes et miniatures (atlas) ; réduction du cold start ; détection des fuites mémoire ; stabilisation et fuzzing.

**Critères d'acceptation** — un rapport de benchmark reproductible publié dans `docs/benchmarks/` pour chaque release ; les cibles de 5.2 sont atteintes ou un écart documenté est accepté ; absence de fuite significative ; régressions bloquées en CI lorsque possible.

#### Tranche 16 — Extensibilité et fonctions avancées (v2)

**Contenu** — plugins WASM (`fx-plugin-api`/`fx-plugin-host`) avec API versionnée et permissions déclarées ; SDK et exemples ; scripting Rhai ; gestionnaire d'associations de fichiers ; comparaison de dossiers (diff) ; dossiers virtuels (smart folders) ; suppression sécurisée ; reprise de transferts ; restauration de versions ; éditeur d'ACL.

**Critères d'acceptation** — un plugin d'exemple ajoute une action à la palette sans recompiler ; un plugin sans permission réseau ne peut pas en utiliser ; crash d'un plugin isolé ; permissions visibles ; API documentée.

#### Tranche 17 — Packaging et distribution

**Contenu** — MSI (`cargo-wix`) et MSIX ; version portable en ZIP ; signature du code ; mise à jour signée via GitHub Releases (rollback, changelog, SHA-256) ; canaux stable/bêta/nightly ; SBOM ; métadonnées, icône, notes de version générées.

**Critères d'acceptation** — un utilisateur sans connaissance de Rust installe et lance Ferrix en moins de deux minutes ; désinstallation propre ; mise à jour d'une version à l'autre sans perte de configuration ; artefacts signés et vérifiés ; SBOM édité.

#### Tranche 18 — Release 1.0

**Contenu** — documentation utilisateur (installation, raccourcis, FAQ) ; `CONTRIBUTING.md`, `SECURITY.md`, guide d'architecture ; liste des « good first issue » (backlog `Community`) ; bêta publique (FR/EN) ; annonce (GitHub, Reddit r/rust, r/Windows10, Hacker News).

**Critères d'acceptation** — bêta testée par au moins 20 utilisateurs sur des configurations variées ; aucun bug bloquant ouvert, bugs majeurs corrigés ; benchmarks de la release publiés ; aucune vulnérabilité critique connue ; validation Windows 10 et 11 ; documentation complète.

---

## 17. Risques techniques

| Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|
| Licence de Slint incompatible avec la licence visée | Élevée | Élevé | Décision en Tranche 0 (prototypes Slint et iced), ADR documenté (9.1) |
| Framework UI insuffisant (virtualisation, UIA) | Moyenne | Très élevé | Spike et ADR avant développement massif (section 11) |
| Accessibilité incomplète ajoutée trop tard | Élevée | Très élevé | Démarrage Tranche 0, tests continus, audits Narrator/NVDA réels (D-3) |
| Complexité de l'interop COM Shell (`IContextMenu`, `IFileOperation`) | Élevée | Élevé | Temps dédié Tranche 3, prototypes isolés dans `fx-shell`, tests manuels documentés |
| Extensions Shell tierces lentes ou instables | Élevée | Élevé | Timeouts, mesure d'ouverture, blocage, isolation, fallback (13.8) |
| Corruption pendant opérations | Moyenne | Critique | APIs système (`IFileOperation`), transactions, journaux, tests de robustesse (15.4) |
| USN Journal nécessite des privilèges admin sur certains volumes | Moyenne | Moyen | Repli `jwalk` (SRC-05), message clair à l'utilisateur |
| Index corrompu | Moyenne | Moyen | Reconstruction, versionnage, sauvegarde avant migration |
| CfApi peu documentée côté Rust | Élevée | Moyen | Wrapper manuel dans `fx-cloud`, tests sur machines réelles avec OneDrive |
| Google Drive et Dropbox changent clients locaux ou APIs | Moyenne | Moyen | Abstraction `CloudProvider`, repli API, tests de non-régression |
| Licence LZTurbo incompatible avec l'OSS | Certaine | Faible (si respectée) | Jamais inclus par défaut, plugin opt-in, `docs/LEGAL.md` (9.2) |
| Licence UnRAR restrictive | Certaine | Faible | Lecture seule, feature `rar`, désactivable (9.3) |
| UI Automation difficile à rendre correct | Élevée | Élevé | Démarrage Tranche 0, tests réels Narrator/NVDA, pas seulement une case cochée |
| Indexation sur très gros volumes (> 5 M fichiers) | Moyenne | Moyen | Indexation incrémentale, progression visible, jamais bloquante |
| Licences de codecs | Élevée | Élevé | Audit, composants optionnels, SBOM, `cargo deny` en CI |
| Scope trop large | Élevée | Critique | Tranches et critères d'acceptation stricts, backlog v1.5/v2 assumé |
| Positionnement « pure Rust » irréaliste | Moyenne | Moyen | Rust-first transparent (D-7) |
| Performance réseau imprévisible | Élevée | Élevé | Timeouts, annulation, async, aucun blocage UI (13.17) |
| ARM64 incomplet | Moyenne | Moyen | CI ARM64 périodique, inventaire des dépendances |
| Mises à jour Windows modifiant le comportement Shell | Moyenne | Moyen | Matrice de tests sur builds Windows récents, CI `windows-latest` |
| Maintenance d'un crate RAW ou d'un binding non actif | Moyenne | Faible | Choix de crates actives, abstraction via trait `PreviewProvider` |
| Charge de travail sous-estimée (projet très large) | Élevée | Élevé | Découpage en tranches, MVP strict, backlog `v1.5`/`v2` assumé |

---

## 18. Conventions de développement et définition de « terminé »

### 18.1 Définition de « terminé » (DoD) pour chaque tranche

- [ ] Critères d'acceptation de la tranche validés (tests automatisés ou manuels documentés).
- [ ] `cargo fmt`, `cargo clippy --all-targets --all-features -- -D warnings`, tests verts en CI.
- [ ] Tests unitaires sur la logique métier (`fx-domain`, `fx-fs`, `fx-ops`, `fx-index`, `fx-codec`, `fx-preview`).
- [ ] Opérations longues annulables ; aucun blocage du thread UI ; scénarios d'échec testés.
- [ ] Vérification UIA : chaque nouveau contrôle a un nom et un rôle, testé avec Narrator au minimum.
- [ ] Vérification clavier : aucune action nouvelle n'est uniquement accessible à la souris.
- [ ] Logs sans secret ni contenu de fichier ; chemins anonymisés dans les rapports exportés.
- [ ] Benchmarks de non-régression exécutés si un chemin chaud a été touché.
- [ ] Textes ajoutés dans les fichiers de traduction ; documentation utilisateur mise à jour.
- [ ] `docs/LEGAL.md` mis à jour si une dépendance a été ajoutée ; licence validée.
- [ ] ADR créé pour toute décision d'architecture structurante ; limitations documentées.
- [ ] Checklist de validation manuelle Windows produite ; Changelog mis à jour.

### 18.2 Code et qualité

- Tests unitaires là où c'est possible ; tests d'intégration manuels documentés pour `fx-shell` (le COM est difficile à simuler).
- `unsafe` interdit par défaut : autorisé uniquement dans les crates d'interop, avec commentaire `// SAFETY:` et test ciblé.
- Pas de `unwrap()` ni `expect()` hors tests et initialisation (invariant documenté accepté) ; erreurs typées (`thiserror`) dans les bibliothèques, `anyhow` uniquement dans les binaires, avec contexte.
- Aucun blocage de la boucle UI : tout I/O passe par le runtime ou un pool.
- Ne jamais prétendre qu'une API Windows est supportée sans validation sur Windows réel.

### 18.3 Git

- Conventional Commits : `feat:`, `fix:`, `perf:`, `a11y:`, `docs:`, `refactor:`, `test:`, `build:`, `ci:`, `security:`.
- Une tranche = une branche = une ou plusieurs PR, jamais de mélange de tranches ; revue obligatoire avant fusion sur `main`.
- Petits commits cohérents.

### 18.4 Documentation

- README dans chaque crate, rustdoc sur toutes les API publiques.
- ADR numérotés dans `docs/adr/`.
- Documentation utilisateur dans `docs/user/` ; `ARCHITECTURE.md`, `SECURITY.md`, `PRIVACY.md`, `CONTRIBUTING.md`, `BENCHMARKS.md`.

### 18.5 Licences

- Licence du projet : MIT OU Apache-2.0 (sous réserve de la décision 9.1).
- Toute dépendance non permissive listée dans `docs/LEGAL.md` avec son statut : **par défaut**, **optionnelle**, **isolée** ou **opt-in**.
- `cargo deny` bloque en CI toute licence non autorisée sans exception documentée.

### 18.6 Processus de validation manuelle

Chaque tranche se termine par une checklist de validation manuelle sur Windows réel (au minimum : une machine Win10 22H2 et une Win11), listant les scénarios à rejouer et ce qui reste à tester manuellement. Le travail d'un agent IA suit la même checklist (section 20).

---

## 19. Packaging, licences et indicateurs

### 19.1 Packaging et mises à jour

**Formats** — MSI (utilisateur) ; MSIX si les restrictions sont acceptables ; ZIP portable (marqueur de mode portable, paramètres en local).

**Fonctions** — installation par utilisateur ; installation machine optionnelle ; désinstallation propre ; association optionnelle de fichiers ; mise à jour signée avec rollback ; canaux stable, bêta et nightly ; changelog ; sommes SHA-256 ; signatures des artefacts ; **jamais d'auto-update non signé**.

**Métadonnées** — icône, manifeste, notes de version générées.

### 19.2 Licences et dépendances

Le projet maintient `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD_PARTY_NOTICES.md`, `docs/LEGAL.md` et un **SBOM par release**. Chaque dépendance est examinée selon : licence, activité du projet, vulnérabilités, code natif, compatibilité ARM64, taille, politique de mise à jour, nécessité réelle. Les composants sensibles (LZTurbo, RAR, FFmpeg, PDFium, SDK) font l'objet d'un ADR et d'une vérification juridique avant distribution (section 9).

### 19.3 Indicateurs de succès

**Produit** — taux de crash ; temps de démarrage ; latence de navigation ; latence de recherche ; nombre d'erreurs d'opérations ; consommation mémoire ; taux de réussite des restaurations ; nombre d'utilisateurs actifs (uniquement si mesure opt-in) ; satisfaction des testeurs.

**Qualité** — couverture des modules critiques ; vulnérabilités ouvertes ; défauts d'accessibilité ; régressions de performance ; temps moyen de résolution ; pourcentage de dépendances à jour.

---

## 20. Utilisation avec un agent IA (Claude Code et équivalents)

### 20.1 Fichier `CLAUDE.md` à la racine du dépôt (à créer en Tranche 0)

```markdown
# CLAUDE.md — Règles pour l'agent

## Contexte
- Projet : Ferrix Explorer, explorateur Windows 10/11 en Rust.
- Source de vérité : PRD.md (ce document) et docs/adr/.

## Règles de travail
1. Toujours lire le PRD et la tranche en cours avant de coder.
2. Proposer d'abord la structure des fichiers et modules, puis attendre validation si le changement est structurant.
3. Une tranche = une branche. Ne jamais implémenter une fonctionnalité d'une tranche ultérieure.
4. Aucune dépendance ajoutée sans vérifier sa licence et mettre à jour docs/LEGAL.md.
5. Aucun backend non permissif activé par défaut.
6. Tout `unsafe` a un commentaire // SAFETY: et un test, et reste dans les crates d'interop.
7. Le COM et le Win32 bas niveau restent dans fx-shell, fx-terminal, fx-git, fx-a11y.
8. fx-domain, fx-fs et fx-ops ne dépendent jamais de fx-ui.
9. Aucun I/O bloquant sur le thread UI ; toute tâche longue est annulable.
10. Chaque contrôle UI a un nom et un rôle accessibles.
11. Exécuter cargo fmt, cargo clippy -D warnings et cargo test avant chaque commit.
12. Commits au format Conventional Commits.
13. Ne jamais prétendre qu'une API Windows est supportée sans validation sur Windows réel.
14. Ne pas commencer la tranche suivante sans validation des critères d'acceptation.
```

### 20.2 Méthode de travail par tranche

Pour chaque tranche : 1) lire intégralement le PRD et les ADR ; 2) inspecter l'état réel du dépôt et en résumer l'état ; 3) lister les exigences et critères d'acceptation de la tranche ; 4) identifier les risques techniques et les dépendances (avec licences) ; 5) proposer la structure de modules et un plan d'implémentation ; 6) signaler les risques avant de coder ; 7) créer ou mettre à jour les ADR nécessaires ; 8) implémenter en petits commits cohérents avec tests ; 9) exécuter `cargo fmt`, `cargo clippy`, `cargo test` ; 10) documenter les limitations ; 11) produire une checklist de validation manuelle Windows ; 12) ne pas démarrer la tranche suivante sans validation.

### 20.3 Prompt type par tranche

```text
Tu travailles sur Ferrix Explorer, un explorateur de fichiers Windows Rust-first.

Lis intégralement PRD.md et les ADR du dépôt. Nous sommes à la tranche [NUMÉRO ET NOM].

Avant de modifier le code :
1. inspecte le workspace et résume son état ;
2. liste les exigences de la tranche (IDs du backlog et critères d'acceptation) ;
3. identifie les risques techniques et les dépendances avec leur licence ;
4. propose la structure de modules et un plan d'implémentation ;
5. indique les critères d'acceptation vérifiables.

Ensuite, implémente la tranche par étapes atomiques avec des tests.

Contraintes :
- ne jamais bloquer le thread UI ; utiliser l'annulation pour les tâches longues ;
- préserver l'accessibilité (nom/rôle sur chaque contrôle) ;
- limiter et documenter tout code unsafe ;
- ne pas ajouter de dépendance sans vérifier licence et maintenance (docs/LEGAL.md) ;
- exécuter cargo fmt, cargo clippy et cargo test ;
- ne pas prétendre qu'une API Windows est supportée sans validation ;
- ne pas commencer la tranche suivante ;
- documenter les limitations et la procédure de test manuel Windows.

À la fin, fournis : le résumé des changements ; les fichiers modifiés ; les commandes de
validation ; les résultats des tests ; les limitations restantes ; la checklist d'acceptation.
```

### 20.4 Prompts de démarrage recommandés

- **Tranche 0** : « Crée le workspace multi-crates de la section 10.1, configure la CI et ouvre une fenêtre vide avec un élément racine UI Automation. Prépare les deux prototypes UI (Slint et iced) et les spikes COM/UIA/DnD pour comparaison, sans trancher la décision UI. »
- **Tranche 1** : « Implémente le listing asynchrone dans `fx-fs` et la vue liste virtualisée dans `fx-ui`. Objectif : 50 000 entrées fluides. Ajoute les tests et un benchmark. »

---

## 21. Glossaire

| Terme | Définition |
|---|---|
| **USN Journal** | Journal des modifications d'un volume NTFS, lisible pour une indexation incrémentale. |
| **MFT** | Master File Table, table des fichiers NTFS, permettant une énumération rapide. |
| **CfApi** | Cloud Files API de Windows, utilisée par les clients de synchronisation pour les fichiers en ligne uniquement. |
| **ConPTY** | API Windows de pseudo-terminal, base du terminal intégré. |
| **UIA** | UI Automation, API d'accessibilité de Windows utilisée par les lecteurs d'écran. |
| **Quick Look** | Aperçu rapide d'un fichier sans l'ouvrir, par barre d'espace. |
| **Dual-pane** | Mode à deux panneaux côte à côte, façon Total Commander. |
| **Backend optionnel** | Moteur de compression ou d'aperçu installé et activé explicitement par l'utilisateur. |
| **Placeholder / hydratation** | Fichier cloud représenté localement par un emplacement virtuel, téléchargé à la demande. |
| **Reparse point** | Point de redirection NTFS (liens, montages, placeholders) ; risque en parcours récursif. |
| **Plancher acceptable** | Valeur de performance minimale tolérée (voir 5.2) ; au-delà, l'écart doit être documenté et accepté. |
| **Tranche** | Version livrable et testable du projet ; conditionnée à la validation de la tranche précédente. |

---

## Annexe A — Backlog fonctionnel (index de traçabilité)

**Légende**

- **Impact** : 1 (faible) à 5 (critique).
- **Effort** : 1 (trivial) à 5 (très complexe).
- **Priorité** : `MVP` (release 1.0), `v1.5` (post-lancement rapide), `v2` (vision long terme), `Community` (ticket pour contributeurs externes).
- **Tranche** : numéro de la tranche de la section 16. Le détail des exigences de chaque ID est en section 13.

### A.1 Navigation (NAV)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| NAV-01 | Listing asynchrone d'un dossier (jwalk) | 5 | 2 | MVP | 1 |
| NAV-02 | Vue liste et vue détails (colonnes nom, taille, date, type) | 5 | 2 | MVP | 1 |
| NAV-03 | Vues icônes (petite, moyenne, grande, très grande), mosaïque, contenu | 4 | 3 | MVP | 9 |
| NAV-04 | Breadcrumb cliquable avec menu déroulant des sous-dossiers | 5 | 2 | MVP | 1 |
| NAV-05 | Barre d'adresse éditable (saisie directe du chemin, autocomplétion) | 4 | 1 | MVP | 1 |
| NAV-06 | Historique back/forward (Alt+←/→, boutons latéraux de la souris) | 4 | 1 | MVP | 1 |
| NAV-07 | Treeview des dossiers (arborescence) | 3 | 2 | MVP | 4 |
| NAV-08 | Filtrage à la volée (liste filtrée en temps réel) | 4 | 2 | MVP | 1 |
| NAV-09 | Tri multi-critères et groupement (type, date, taille) | 4 | 2 | MVP | 1 et 9 |
| NAV-10 | Zoom Ctrl+molette (taille des icônes) | 4 | 2 | MVP | 9 |
| NAV-11 | Virtualisation des listes (rendu des seuls éléments visibles) | 5 | 3 | MVP | 1 |
| NAV-12 | Chemins longs (> 260 caractères, préfixe `\\?\`) | 4 | 2 | MVP | 1 |
| NAV-13 | Multi-onglets (Ctrl+T, Ctrl+W, Ctrl+Tab, réordonnancement, détachement) | 5 | 3 | MVP | 5 |
| NAV-14 | Multi-fenêtres indépendantes | 4 | 2 | MVP | 5 |
| NAV-15 | Split view (deux panneaux dans un onglet) | 4 | 3 | MVP | 5 |
| NAV-16 | Mode dual-pane façon Commander (copie vers le panneau opposé, navigation synchronisée optionnelle) | 4 | 3 | MVP | 5 |

### A.2 Sidebar (SID)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| SID-01 | Épinglés (ajout, retrait, réordonnancement par glisser-déposer) | 4 | 2 | MVP | 4 |
| SID-02 | Accès rapide (dossiers fréquents, calcul dynamique) | 3 | 2 | MVP | 4 |
| SID-03 | Récents (fichiers et dossiers, stockés en SQLite) | 4 | 2 | MVP | 4 |
| SID-04 | Bibliothèques (Documents, Images, Musique, Vidéos, Téléchargements via KnownFolders) | 4 | 1 | MVP | 4 |
| SID-05 | Ce PC : lecteurs locaux avec jauge d'espace libre/utilisé | 4 | 2 | MVP | 4 |
| SID-06 | Lecteurs réseau mappés (reconnexion automatique) | 3 | 2 | MVP | 4 |
| SID-07 | Détection des lecteurs amovibles (hotplug USB) | 3 | 2 | MVP | 4 |
| SID-08 | Workspaces sauvegardés (ensembles d'onglets et de panneaux nommés) | 3 | 2 | MVP | 5 |
| SID-09 | Section « Comptes cloud » avec statut et quota | 4 | 3 | v1.5 | 11 |
| SID-10 | Découverte réseau SMB (voisinage réseau) | 3 | 3 | v1.5 | 14 |

### A.3 Opérations sur fichiers (OPS)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| OPS-01 | Copier, couper, coller (presse-papiers Windows, CF_HDROP et Preferred DropEffect) | 5 | 3 | MVP | 2 |
| OPS-02 | Renommer en ligne (F2) | 5 | 2 | MVP | 2 |
| OPS-03 | Renommage par lot avec regex (aperçu avant application) | 4 | 2 | MVP | 7 |
| OPS-04 | Supprimer vers la Corbeille, et suppression définitive (Maj+Suppr) | 5 | 2 | MVP | 2 |
| OPS-05 | Annuler / rétablir (undo/redo) des opérations de l'application | 4 | 3 | MVP | 2 |
| OPS-06 | Glisser-déposer interne (déplacer, copier) | 5 | 3 | MVP | 2 |
| OPS-07 | Glisser-déposer OLE vers et depuis d'autres applications | 4 | 4 | MVP | 3 |
| OPS-08 | Opérations via IFileOperation (boîte de progression native) | 4 | 3 | MVP | 3 |
| OPS-09 | Boîte de conflit (remplacer, garder les deux, ignorer, annuler) | 4 | 2 | MVP | 2 |
| OPS-10 | Créer dossier et fichier (Ctrl+Maj+N) | 4 | 1 | MVP | 2 |
| OPS-11 | Sélection multiple (Ctrl/Maj+clic, sélection par rectangle) | 5 | 2 | MVP | 2 |
| OPS-12 | Checksums MD5, SHA-256, BLAKE3 (menu et panneau) | 3 | 1 | MVP | 2 |
| OPS-13 | Nettoyeur de dossiers vides | 2 | 1 | MVP | 2 |
| OPS-14 | Propriétés natives (ShellExecuteEx, verbe `properties`) | 4 | 1 | MVP | 3 |
| OPS-15 | « Ouvrir avec » natif | 3 | 1 | MVP | 3 |
| OPS-16 | Compression et décompression (ZIP, 7z, TAR…) via `fx-codec` | 4 | 3 | MVP | 8 |
| OPS-17 | File d'attente d'opérations (pause, reprise, réordonnancement, annulation) | 4 | 3 | v1.5 | 12 |
| OPS-18 | Vérification d'intégrité après copie (hash source/destination) | 3 | 2 | v1.5 | 12 |
| OPS-19 | Détecteur de doublons (par hash, pas seulement par nom) | 4 | 3 | v1.5 | 12 |
| OPS-20 | Montage d'images ISO et VHD/VHDX | 2 | 1 | v1.5 | 12 |
| OPS-21 | Reprise de transferts interrompus | 3 | 4 | v2 | 16 |
| OPS-22 | Suppression sécurisée (écrasement multi-passes) | 2 | 2 | v2 | 16 |
| OPS-23 | Restauration de versions précédentes (Volume Shadow Copy / Historique des fichiers) | 3 | 3 | v2 | 16 |
| OPS-24 | Comparaison de fichiers et dossiers (diff, synchronisation visuelle) | 3 | 3 | v2 | 16 |

### A.4 Quick Look et aperçus (PRV)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| PRV-01 | Aperçu instantané par barre d'espace (cadre, navigation, plein écran) | 5 | 4 | MVP | 8 |
| PRV-02 | Images (formats courants) et RAW (vignette embarquée prioritaire) | 4 | 3 | MVP | 8 |
| PRV-03 | PDF (pages, zoom, navigation) | 4 | 3 | MVP | 8 |
| PRV-04 | Vidéo et audio : vignette, lecture, waveform | 4 | 4 | MVP | 8 |
| PRV-05 | Code source et texte avec coloration syntaxique | 3 | 2 | MVP | 8 |
| PRV-06 | Listing des archives (sans extraction) | 4 | 2 | MVP | 8 |
| PRV-07 | Polices (aperçu avec texte d'exemple, DirectWrite) | 2 | 2 | MVP | 8 |
| PRV-08 | Documents Office via Shell Preview Handlers | 3 | 2 | MVP | 8 |
| PRV-09 | Panneau de métadonnées EXIF et média | 3 | 2 | v1.5 | 12 |
| PRV-10 | Navigation dans les archives comme dossier virtuel (sans extraction) | 4 | 2 | v1.5 | 12 |

### A.5 Organisation (ORG)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| ORG-01 | Barre de statut enrichie (sélection, taille totale, espace libre) | 3 | 1 | MVP | 2 |
| ORG-02 | Tags colorés indépendants de l'arborescence | 3 | 3 | v1.5 | 12 |
| ORG-03 | Treemap d'usage disque (visualisation intégrée) | 4 | 3 | v1.5 | 12 |
| ORG-04 | Dossiers virtuels (smart folders, basés sur des critères) | 3 | 3 | v2 | 16 |
| ORG-05 | Règles d'organisation automatique (type Hazel) | 2 | 3 | Community | — |

### A.6 Recherche (SRC)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| SRC-01 | Barre de recherche globale (Ctrl+F et saisie directe) | 5 | 3 | MVP | 6 |
| SRC-02 | Lecture de l'USN Journal et de la MFT NTFS pour indexation initiale | 5 | 4 | MVP | 6 |
| SRC-03 | Index tantivy avec mise à jour en direct (USN) | 5 | 4 | MVP | 6 |
| SRC-04 | Filtres avancés (type, taille, date, attributs) | 4 | 2 | MVP | 6 |
| SRC-05 | Repli sur parcours `jwalk` si le volume est inaccessible sans privilèges | 4 | 2 | MVP | 6 |
| SRC-06 | Recherche plein texte dans le contenu (texte, code, PDF extrait) | 3 | 4 | MVP (optionnelle, désactivée par défaut) | 6 |
| SRC-07 | Indicateur de progression d'indexation et contrôle (pause, reprise) | 3 | 2 | MVP | 6 |

### A.7 Power-user (PWR)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| PWR-01 | Palette de commandes (Ctrl+Maj+P) | 5 | 2 | MVP | 7 |
| PWR-02 | Terminal intégré (ConPTY) dans le dossier courant (PowerShell, cmd, WSL) | 4 | 2 | MVP | 7 |
| PWR-03 | CLI compagnon `fx` (`fx open .`, `fx search "*.rs"`) | 4 | 2 | MVP | 7 |
| PWR-04 | Support WSL natif (`\\wsl$\`, distributions listées) | 3 | 2 | MVP | 7 |
| PWR-05 | Raccourcis clavier : parité Explorer et remappage utilisateur | 4 | 2 | MVP | 7 |
| PWR-06 | Badges Git (modifié, ajouté, ignoré) dans les listes | 4 | 3 | v1.5 | 13 |
| PWR-07 | Mode de navigation type vim (hjkl) | 2 | 2 | Community | — |

### A.8 Personnalisation et Windows 11 (UX)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| UX-01 | Thème clair et sombre, synchronisé avec Windows (et surcharge manuelle) | 4 | 2 | MVP | 9 |
| UX-02 | Effet Mica (Win11) et repli propre sur Win10 | 4 | 2 | MVP | 9 |
| UX-03 | Compatibilité Snap Layouts (Win11) | 3 | 1 | MVP | 9 |
| UX-04 | Mode portable (exécutable sur clé USB, données à côté) | 3 | 1 | MVP | 9 |
| UX-05 | Mémorisation de la position et de la taille par écran | 2 | 1 | MVP | 9 |
| UX-06 | Colonnes personnalisables (ajout, retrait, réordonnancement, largeur) | 3 | 2 | MVP | 9 |
| UX-07 | Densité d'affichage (compacte, confortable) | 3 | 1 | MVP | 9 |
| UX-08 | Barre d'outils personnalisable | 2 | 2 | MVP | 9 |
| UX-09 | Jump Lists (clic droit sur l'icône de la barre des tâches) | 3 | 2 | v1.5 | 13 |
| UX-10 | Packs d'icônes personnalisés | 2 | 2 | Community | — |

### A.9 Système (SYS)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| SYS-01 | Support Dev Drive (Windows 11) : détection et bonnes pratiques | 2 | 1 | MVP | 4 |
| SYS-02 | Indicateurs visuels des fichiers compressés et chiffrés | 3 | 1 | MVP | 9 |
| SYS-03 | Menu contextuel natif complet (fusion des `IContextMenu`, extensions tierces) | 5 | 4 | MVP | 3 |
| SYS-04 | Menu contextuel additionnel propre à Ferrix | 3 | 2 | MVP | 3 |
| SYS-05 | Récupération de session après plantage | 3 | 2 | v1.5 | 13 |
| SYS-06 | Élévation UAC ponctuelle (opération admin sans relancer l'application) | 3 | 3 | v1.5 | 13 |
| SYS-07 | Définir Ferrix comme explorateur par défaut (si possible proprement) | 3 | 3 | v1.5 | 13 |
| SYS-08 | Éditeur simplifié des ACL NTFS | 2 | 3 | v2 | 16 |
| SYS-09 | Télémétrie opt-in transparente (rapports de crash anonymes) | 2 | 2 | v1.5 | 13 |

### A.10 Accessibilité (A11Y)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| A11Y-01 | Squelette `fx-a11y` (élément racine UIA, rôles de base) | 5 | 2 | MVP | 0 |
| A11Y-02 | Support UI Automation complet (listes, arbres, onglets, boutons, annonces) | 5 | 4 | MVP | 1 à 10 (continu) |
| A11Y-03 | Navigation 100 % clavier, y compris menus et palette | 4 | 2 | MVP | 1 à 10 (continu) |
| A11Y-04 | Mode contraste élevé (suit le thème système) | 3 | 1 | MVP | 10 |
| A11Y-05 | Mise à l'échelle DPI et taille de texte du système | 4 | 2 | MVP | 9 |
| A11Y-06 | Internationalisation FR et EN | 3 | 2 | MVP | 10 |
| A11Y-07 | Audit avec Narrator et NVDA, documenté | 4 | 2 | MVP | 10 |

### A.11 Cloud (CLD)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| CLD-01 | Trait générique `CloudProvider` (list, status, quota, sync_state) | 4 | 3 | v1.5 | 11 |
| CLD-02 | OneDrive : détection du dossier synchronisé, badges via Cloud Files API | 5 | 4 | v1.5 | 11 |
| CLD-03 | Google Drive : détection du client local, repli sur API Drive v3 | 4 | 4 | v1.5 | 11 |
| CLD-04 | Dropbox : détection du client local, repli sur API | 3 | 3 | v1.5 | 11 |
| CLD-05 | Affichage du quota par compte | 3 | 2 | v1.5 | 11 |
| CLD-06 | Authentification OAuth2 (navigateur système, stockage sécurisé des jetons) | 4 | 3 | v1.5 | 11 |

### A.12 Réseau avancé (NET)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| NET-01 | Client FTP, SFTP et WebDAV navigable comme un dossier | 3 | 4 | v2 | 14 |
| NET-02 | Fichiers hors ligne (cache réseau) | 2 | 3 | Community | — |

### A.13 Extensibilité (EXT)

| ID | Fonctionnalité | Impact | Effort | Priorité | Tranche |
|---|---|---|---|---|---|
| EXT-01 | Plugins WASM sandboxés (API stable et versionnée) | 5 | 5 | v2 | 16 |
| EXT-02 | Scripting Rhai pour actions personnalisées | 3 | 3 | v2 | 16 |
| EXT-03 | Gestionnaire simplifié des associations de fichiers | 2 | 2 | v2 | 16 |
| EXT-04 | Interface de ligne de commande étendue (scripts, sorties JSON) | 3 | 2 | v1.5 | 7 (base) et 13 |

### A.14 Synthèse des priorités

| Priorité | Périmètre |
|---|---|
| **MVP** | Majorité de NAV, SID (hors cloud), OPS (hors v2), PRV, PWR, UX, A11Y — release 1.0 (tranches 0 à 10, 17, 18) |
| **v1.5** | Cloud, outils avancés, Git, Jump Lists, SMB — tranches 11 à 14 |
| **v2** | Plugins WASM, Rhai, diff, smart folders, FTP/SFTP, ACL, versions — tranches 16 |
| **Community** | Règles automatiques, vim-like, packs d'icônes, fichiers hors ligne — tickets ouverts |

---

## Annexe B — Matrice « killer features » (synthèse)

> Extraite du PRD sonnet : cadrage rapide des features différenciantes. En cas d'écart avec l'Annexe A, l'Annexe A fait foi.

| Feature | Impact | Effort | Priorité |
|---|---|---|---|
| Palette de commandes (Ctrl+Maj+P) | 5 | 2 | **MVP** |
| Quick Look / aperçu rapide (Espace) | 5 | 4 | **MVP** (Tranche 8 — décision D-1) |
| Terminal intégré (ConPTY) | 4 | 2 | **MVP** |
| CLI compagnon (`fx open .`) | 4 | 2 | **MVP** |
| Dual-pane façon Commander | 4 | 3 | **MVP** (Tranche 5) |
| Support WSL natif | 3 | 2 | **MVP** |
| Git status badges | 4 | 3 | v1.5 |
| Workspaces sauvegardés | 3 | 2 | **MVP** |
| Support UI Automation (lecteurs d'écran) | 5 | 4 | **MVP — démarré dès la Tranche 0** |
| Plugins WASM | 5 | 5 | v2 (gros chantier, bon pour attirer des contributeurs) |

**Note de version (PRD sonnet) :** l'accessibilité (Tranche 10) et les quick wins power-user (Tranche 7) ont été remontés plus tôt que dans les versions initiales, car les ajouter après coup sur une UI déjà figée est bien plus coûteux. Le Quick Look devient sa propre tranche dédiée vu sa complexité et son impact.

---

## Annexe C — Traçabilité des sources et historique du document

### C.1 Contributions par PRD d'origine

| Source | Ce qui a été repris | Où |
|---|---|---|
| **PRD_claude-haiku-5.5-high** | Socle du document, backlog IDé, critères d'acceptation par tranche, objectifs de performance, décisions légales (Slint/LZTurbo/UnRAR), modèle de threading, caches, personas, DoD, CLAUDE.md, glossaire | §5.2, §6, §7, §9, §10.3, §10.8, §16, §18, §20.1, §21, Annexe A |
| **PRD_gpt-5.6-sol** | Exigences fonctionnelles détaillées, modèle de menace et sécurité, stratégie de tests (5 niveaux), table des raccourcis, syntaxe de recherche, isolation des extensions Shell, sécurité des archives, positionnement Rust-first, KPIs, SBOM, packaging, jalons | §4, §10.2/10.4/10.5/10.7, §13, §14.3, §15, §19, §20, Annexe A (découpage crates) |
| **PRD_claude-sonnet-5-high** | Cadrage « killer features », matrice par thème, justification du réordonnancement des tranches, moteur preview/compression | Annexe B, §6 (D-1, D-3), §12 |

### C.2 Historique des versions

| Version | Changements |
|---|---|
| 1.0 | Première version (vision, stack, roadmap initiale) — PRD haiku |
| 2.0 | Matrice impact/effort, moteur Quick Look et backends optionnels, accessibilité anticipée, objectifs de performance mesurables, décisions légales, backlog identifié, critères d'acceptation, conventions et prompts d'agent — PRD haiku |
| — | Déclinaison « 3 parties » (matrice + moteur preview + PRD révisé avec Quick Look en tranche dédiée) — PRD sonnet |
| — | Spécification fonctionnelle complète (41 sections : exigences, sécurité, tests, KPIs, packaging) — PRD gpt-5.6 |
| **3.0** | **Fusion des trois PRD en un document unique** ; conflits arbitrés (section 6) ; restructuration en 22 sections + 3 annexes |

---

**Fin du document — PRD Ferrix Explorer v3.0**

