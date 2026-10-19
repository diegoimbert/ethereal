---
title: projets et enregistrement
order: 40
summary: Créez, ouvrez, renommez et supprimez des projets, partez de modèles, et voyez comment Ethereal enregistre votre travail pour vous.
platform: all
audience: beginner
covers:
  - first-run/the-welcome-screen-is-the-project-picker #the-welcome-screen
  - first-run/closing-the-welcome-screen-drops-you-into-a-draft-project #new-projects-start-as-drafts
  - first-run/untouched-draft-projects-are-thrown-away #new-projects-start-as-drafts
  - first-run/get-started-guided-tours-and-soon-items #guided-tours
  - first-run/starting-a-tour-or-a-new-project-during-a-jam-asks-you-to-le #switching-projects-during-a-live-session
  - first-run/reopen-last-project-on-launch-off-by-default #reopening-your-last-project
  - first-run/project-links-in-the-address-bar-web #project-links-in-the-address-bar
  - first-run/project-templates-and-the-default-template #templates
  - projects-storage/the-project-menu #the-project-menu
  - projects-storage/some-project-actions-are-only-in-the-command-palette #actions-in-the-command-palette
  - projects-storage/recent-projects-search-right-click-menu #recent-projects
  - projects-storage/autosave-and-the-unsaved-changes-dot #saving-is-automatic
  - projects-storage/plugin-tweaks-don-t-show-the-unsaved-dot #plugin-settings-and-the-dot
  - projects-storage/closing-a-web-tab-doesn-t-warn-you #closing-the-browser-tab
  - projects-storage/quitting-the-desktop-app #quitting-the-desktop-app
  - projects-storage/cmd-w-closes-the-top-floating-window-not-the-app #quitting-the-desktop-app
  - projects-storage/deleting-a-project-is-permanent #deleting-a-project
  - projects-storage/files-from-newer-or-pre-release-versions-are-refused #projects-from-a-newer-version
  - projects-storage/local-copy-projects-after-a-jam #local-copy-projects
---

Un projet, c’est un morceau : ses pistes, ses clips, ses appareils et ses réglages. Cette page explique comment créer et retrouver des projets, et comment ils sont enregistrés. Pour savoir où les projets sont stockés (et pourquoi c’est important sur le web), lisez d’abord [où vivent vos projets](/docs/welcome/where-your-projects-live).

## l’écran d’accueil {#the-welcome-screen}

Au démarrage, Ethereal affiche l’écran d’accueil. C’est aussi votre liste de projets (l’écran **ouvrir un projet**). Il contient :

- le logo d’Ethereal et la version que vous utilisez ;
- **rejoindre une session…** pour rejoindre la session en direct de quelqu’un à partir de son lien d’invitation (voir [rejoindre une session](/docs/sharing/joining)) ;
- **importer depuis une autre DAW…** pour ouvrir un fichier Ableton Live, FL Studio, REAPER ou DAWproject (voir [projets d’autres DAW](/docs/import-export/other-daws)) ;
- **nouveau projet** ;
- une liste **pour commencer** de visites guidées ;
- vos **projets récents**.

Rien ne s’ouvre tout seul sans que vous le demandiez (voir [rouvrir votre dernier projet](#reopening-your-last-project)). Vous pouvez rouvrir cet écran plus tard depuis le menu du projet : **ouvrir un projet…**.

### les nouveaux projets commencent en brouillon {#new-projects-start-as-drafts}

Si vous fermez l’écran d’accueil ([[key:Escape]] ou un clic en dehors) sans choisir de projet, vous arrivez dans un **nouveau projet** nommé d’après la date du jour. **Nouveau projet** dans le menu du projet fait la même chose.

Un nouveau projet part de votre [modèle par défaut](#templates). Si vous n’en avez pas défini, vous obtenez le projet de départ : une piste MIDI appelée **synthé**, avec le Synth intégré, prête pour dessiner des notes.

Un nouveau projet reste un **brouillon** tant que vous n’y changez rien (une modification, une étape d’annulation ou un renommage). Si vous quittez un brouillon sans y avoir touché, en ouvrant un autre projet ou en quittant l’application, il est supprimé et n’apparaît jamais dans vos projets récents. Vous pouvez donc fermer l’écran d’accueil juste pour jeter un œil.

### visites guidées {#guided-tours}

La liste **pour commencer** de l’écran d’accueil lance des visites guidées de la vraie application : faire votre premier morceau, jouer avec des amis, enregistrer de l’audio, importer un projet d’une autre DAW. En lancer une crée un projet pour elle et vous guide pas à pas. Quand vous terminez ou quittez la visite, vous revenez à l’écran d’accueil.

Une visite terminée affiche une coche. Cliquer de nouveau dessus vous demande si vous voulez la **recommencer**. Sur un téléphone, les guides sont marqués **bientôt** : les visites se font dans l’application de bureau et la version web.

### rouvrir votre dernier projet {#reopening-your-last-project}

Dans **réglages › général**, sous **démarrage**, **rouvrir le dernier projet au lancement** ouvre le projet sur lequel vous travailliez au démarrage d’Ethereal. Ce réglage est désactivé par défaut.

Le projet n’est rouvert que si Ethereal s’est bien fermé la dernière fois. Après un plantage, c’est vous qui choisissez quoi faire dans la [fenêtre de récupération](/docs/basics/versions-and-backups#after-a-crash). Réglage désactivé, Ethereal ne charge aucun plug-in tant que vous n’avez pas choisi de projet, ce qui est pratique si un plug-in pose problème.

## projets récents {#recent-projects}

L’écran d’accueil liste vos projets, du plus récent au plus ancien. Le projet ouvert en ce moment n’est pas dans la liste.

- **Tapez** dans le champ de recherche pour filtrer la liste par nom. [[key:Enter]] ouvre le premier résultat.
- **Cliquez** sur un projet pour l’ouvrir.
- **Faites un clic droit** sur un projet, ou utilisez son bouton **⋯**, pour plus d’options : **ouvrir**, **ouvrir sans plug-ins** (voir [le mode sans échec](/docs/advanced/safe-mode)), **renommer**, **dupliquer**, **exporter…**, des actions de partage pour les projets partagés, et **supprimer…**.

## le menu du projet {#the-project-menu}

Le bouton **☰** en haut à gauche ouvre le menu du projet :

- **nouveau projet**, **ouvrir un projet…** et **rejoindre une session…** ;
- **enregistrer comme modèle…** ;
- **gamme du projet** (voir [la gamme du projet](/docs/basics/tempo-and-time-signature#the-project-scale)) ;
- **importer un projet…** et **importer depuis une autre DAW ▸** ;
- **exporter le projet…**, **exporter vers une autre DAW ▸** et **exporter l’audio…** ;
- **versions…** et **enregistrer un instantané…** (voir [versions et sauvegardes](/docs/basics/versions-and-backups)) ;
- **résoudre les appareils et échantillons manquants…**, seulement quand il manque quelque chose que le projet utilise (voir [projets d’autres DAW](/docs/import-export/other-daws)) ;
- **supprimer le projet…** ;
- **nouveautés…**, et dans l’application de bureau **rechercher des mises à jour…**.

Pour renommer le projet, cliquez sur son nom à côté du bouton ☰, tapez le nouveau nom et appuyez sur [[key:Enter]].

### actions de la palette de commandes {#actions-in-the-command-palette}

Quelques actions sur le projet ne se trouvent que dans la [palette de commandes](/docs/basics/command-palette-and-settings) ([[action:palette.open]]) :

- **enregistrer sous…** : enregistre une copie du projet, médias compris, sous un nouveau nom, et passe sur la copie ;
- **dupliquer le projet** : crée une copie et vous laisse où vous êtes ;
- **renommer le projet** ;
- **nouveau projet depuis un modèle…** ;
- **relier les échantillons manquants…** et **tout rassembler et enregistrer** (voir [déplacer des projets](/docs/basics/moving-projects#missing-samples)).

Dupliquer et renommer se trouvent aussi dans le menu ⋯ d’un projet récent.

## modèles {#templates}

Un modèle est un point de départ pour de nouveaux projets.

- **Enregistrer comme modèle…** dans le menu du projet enregistre le projet entier (pistes, appareils, états des plug-ins et réglages) comme modèle de projet. Ses échantillons sont copiés dans votre bibliothèque, pour que le modèle fonctionne encore après la suppression du projet.
- **Nouveau projet depuis un modèle…** dans la palette de commandes vous permet de nommer un nouveau projet et de choisir son point de départ : **projet vide** ou l’un de vos modèles.
- Dans cette liste, faites un clic droit sur un modèle et choisissez **utiliser pour les nouveaux projets** pour en faire votre modèle **par défaut**. Dès lors, chaque nouveau projet, y compris celui où vous arrivez en fermant l’écran d’accueil, part de ce modèle. **Ne plus utiliser par défaut** revient au projet de départ.

## l’enregistrement est automatique {#saving-is-automatic}

Il n’y a pas d’étape d’enregistrement à retenir. Ethereal enregistre le projet environ une seconde après votre dernière modification. Tant que des modifications ne sont pas encore enregistrées, un **point** s’affiche après le nom du projet dans la barre du haut.

[[action:project.save]] enregistre immédiatement. Le raccourci fonctionne partout, même pendant que vous tapez dans un champ de texte.

### réglages des plug-ins et le point {#plugin-settings-and-the-dot}

Tourner un bouton dans la fenêtre propre d’un plug-in n’affiche pas le point, car l’application ne voit pas l’intérieur du plug-in. Ethereal enregistre quand même les réglages des plug-ins en arrière-plan, un peu plus lentement. [[action:project.save]] lit tout de suite les réglages actuels de chaque plug-in, et quitter l’application de bureau les enregistre aussi.

### fermer l’onglet du navigateur [[web]] {#closing-the-browser-tab}

Dans le navigateur, fermer l’onglet n’affiche aucun avertissement « quitter cette page ? ». Quand vous masquez ou fermez l’onglet, Ethereal enregistre immédiatement toute modification, mais un navigateur ne peut attendre qu’un instant. Par prudence, attendez que le point après le nom du projet ait disparu avant de fermer l’onglet.

### quitter l’application de bureau [[desktop]] {#quitting-the-desktop-app}

Quitter ([[key:Mod+Q]], ou fermer la fenêtre principale) demande **quitter Ethereal ?** et vous indique que vos modifications sont enregistrées avant la fermeture. [[key:Enter]] confirme. Les réglages des plug-ins sont aussi enregistrés ; cela peut prendre quelques secondes avec beaucoup de plug-ins (30 secondes au plus). Si le projet ne peut toujours pas être enregistré, une copie est gardée dans le dossier **recovered** de l’application, et le lancement suivant vous indique où elle se trouve.

[[key:Mod+W]] ne quitte pas : il ferme la fenêtre flottante du dessus, comme la fenêtre d’un appareil ou d’un plug-in.

## supprimer un projet {#deleting-a-project}

**Supprimer le projet…** (dans le menu du projet, ou le menu d’un projet récent) demande une confirmation, puis supprime le projet avec toutes ses [versions](/docs/basics/versions-and-backups).

> [!warning]
> La suppression d’un projet est définitive. Il n’y a ni corbeille ni annulation. Si vous risquez d’en avoir besoin un jour, [exportez-le](/docs/basics/moving-projects#exporting-a-project) d’abord.

Si vous supprimez le projet ouvert, Ethereal passe d’abord à votre autre projet le plus récent, ou à un nouveau brouillon s’il n’y en a pas.

## changer de projet pendant une session en direct {#switching-projects-during-a-live-session}

Si vous êtes dans une [session en direct](/docs/sharing/live-sessions), ouvrir, créer, importer ou supprimer un projet, ou lancer une visite guidée, vous demande d’abord si vous voulez **quitter la session**. Le bouton de confirmation dit ce qui se passe ensuite : **quitter et ouvrir**, **quitter et créer**, etc. Votre copie du projet partagé reste dans vos projets, et vous pourrez rejoindre la session plus tard.

### projets en copie locale {#local-copy-projects}

Quand rejoindre une session en direct remplace un projet que vous aviez déjà (par exemple, vous rejoignez à nouveau une session à laquelle vous aviez participé), votre version précédente est gardée comme projet séparé avec un badge **copie locale**. Voir [les sessions en direct](/docs/sharing/live-sessions).

## liens de projet dans la barre d’adresse [[web]] {#project-links-in-the-address-bar}

Dans le navigateur, l’adresse d’un projet ouvert ressemble à `/p/my-song/<id>`. Recharger la page rouvre ce projet, et les boutons précédent et suivant du navigateur passent d’un projet ouvert à l’autre.

Le lien ne fonctionne que dans le navigateur qui contient le projet. Ouvert ailleurs, il affiche la liste des projets avec **ce projet n’est pas enregistré dans ce navigateur**. Pour partager un morceau, utilisez plutôt [le partage](/docs/sharing/overview).

## projets d’une version plus récente {#projects-from-a-newer-version}

Un projet enregistré par une version plus récente d’Ethereal ne s’ouvre pas dans une version plus ancienne : vous verrez **ce fichier a été créé avec une version plus récente d’Ethereal**, et vous devrez faire la mise à jour. Les projets enregistrés par des préversions d’Ethereal (antérieures à la première version publique) ne peuvent pas être ouverts du tout.

## et ensuite {#where-next}

- [Versions et sauvegardes](/docs/basics/versions-and-backups)
- [Déplacer des projets entre ordinateurs](/docs/basics/moving-projects)
- [Annulation et historique](/docs/basics/undo-and-history)
