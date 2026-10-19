---
title: plug-ins
order: 20
summary: Utilisez vos plug-ins CLAP, VST3 et Audio Unit dans l’application de bureau, et protégez vos projets quand un plug-in plante ou est manquant.
platform: desktop
audience: everyone
covers:
  - plugins/plugins-are-desktop-only #plugins-need-the-desktop-app
  - plugins/plugin-folders-and-scanning #where-ethereal-looks-for-plugins
  - plugins/failed-plugins-and-retry #plugins-that-failed-to-load
  - plugins/same-plugin-in-several-formats #the-same-plugin-in-several-formats
  - plugins/plugin-editor-windows #opening-a-plugins-window
  - plugins/linux-plugin-editors-need-x11 #plugin-windows-on-linux
  - plugins/keys-in-plugin-windows #keys-in-plugin-windows
  - plugins/sandboxing-and-crashes #when-a-plugin-crashes
  - plugins/open-without-plugins-safe-mode #opening-a-project-without-plugins
  - plugins/missing-plugins #missing-plugins
  - plugins/plugin-state-is-saved-in-the-project #how-plugin-sounds-are-saved
  - plugins/plugin-presets #plugin-presets
---

L’application de bureau d’Ethereal fait tourner, sous forme de plug-ins, les instruments et effets que vous possédez déjà. Cette page explique où Ethereal les trouve, comment ouvrir leurs fenêtres, et ce qui se passe quand un plug-in plante ou n’est pas installé. Pour ajouter un plug-in à une piste, utilisez les mêmes boutons **+** que pour les appareils intégrés (voir [appareils et presets](/docs/making-music/devices-and-presets)).

## les plug-ins demandent l’application de bureau {#plugins-need-the-desktop-app}

L’application de bureau accueille les plug-ins dans ces formats :

- **CLAP**
- **VST3**
- **AU** (Audio Units, sur macOS uniquement)

La version web ne peut pas charger de plug-ins. Sa bibliothèque d’appareils ne liste que les appareils intégrés d’Ethereal, et son onglet **réglages › plug-ins** indique simplement que les plug-ins sont chargés par l’application de bureau. Si vous ouvrez dans le navigateur un projet qui utilise des plug-ins, ces appareils sont contournés et les pistes jouent sans eux.

## découvrir des plug-ins : plug-ins gratuits {#explore-plugins-free-plugins}

Pas encore de plug-ins ? **découvrir des plug-ins** liste des instruments et effets gratuits que vous pouvez ajouter à Ethereal. Ouvrez-le avec le bouton **découvrir des plug-ins** en bas de la bibliothèque d’appareils (la liste qui s’ouvre depuis le bouton **+** d’une chaîne), ou depuis **réglages › plug-ins**.

Cherchez par nom, ou filtrez par catégorie : **synthé**, **effet**, **sampler**, **batterie** ou **utilitaire**. Chaque carte indique l’auteur du plug-in, ses formats, sa licence et la taille du téléchargement, avec des liens vers son site, son code source et sa licence. La liste a deux parties :

- **open source** : dans l’application de bureau, cliquez sur **installer**. Ethereal télécharge le plug-in, vérifie que le fichier est bien celui que liste le catalogue signé, l’installe dans vos dossiers de plug-ins et relance l’analyse, pour qu’il apparaisse tout de suite dans la bibliothèque d’appareils. Les plug-ins installés affichent **installé** avec un bouton **désinstaller**, et **mettre à jour** quand une version plus récente existe. Vous pouvez annuler pendant le téléchargement. Un plug-in qui n’est pas disponible pour votre système indique pourquoi.
- **gratuiciel** : **télécharger** ouvre la page de l’auteur dans votre navigateur. Installez-le comme l’auteur l’explique, puis **réanalyser** dans **réglages › plug-ins**.

Sur le web [[web]], la liste est en lecture seule : les plug-ins ne tournent que dans l’application de bureau, et la fenêtre renvoie vers la page de téléchargement.

## où ethereal cherche les plug-ins {#where-ethereal-looks-for-plugins}

Ouvrez les **réglages** et allez dans l’onglet **plug-ins**.

- **dossiers système** : les dossiers de plug-ins standard de votre ordinateur, une ligne par format. Un dossier qui n’existe pas est marqué **introuvable**. L’interrupteur **analyser** est activé par défaut. Le désactiver ignore ces dossiers et, sur macOS, les Audio Units.
- **vos dossiers** : cliquez sur **ajouter un dossier…** pour analyser un dossier à vous, par exemple un disque où vous gardez des plug-ins. Chaque dossier peut être limité à un format, ou laissé sur **tous les formats**. Cliquez sur **×** pour retirer un dossier.

Ajouter, retirer ou modifier un dossier lance une analyse. Vous pouvez aussi analyser à la main :

- **réanalyser** ne charge que les plug-ins nouveaux ou modifiés depuis la dernière analyse. C’est rapide.
- **réanalyse complète** recharge tous les plug-ins, y compris ceux qui avaient échoué.

Chaque plug-in est analysé dans son propre processus, plusieurs à la fois. Un plug-in qui plante ou se bloque pendant son analyse ne peut pas faire tomber l’application : il est simplement listé comme en échec.

### plug-ins qui n’ont pas pu être chargés {#plugins-that-failed-to-load}

Quand certains plug-ins n’ont pas pu être chargés, la ligne d’analyse indique combien et affiche **plug-ins en échec (n)…**. La liste donne le chemin de chaque plug-in et la raison de l’échec. Faites un clic droit sur un plug-in et choisissez **réessayer…** pour l’analyser à nouveau avec un délai plus long : 90 secondes par défaut, jusqu’à une heure. Les grosses bibliothèques d’échantillons et les plug-ins « shell » qui regroupent des centaines de plug-ins peuvent prendre plusieurs minutes.

## le même plug-in dans plusieurs formats {#the-same-plugin-in-several-formats}

Si un plug-in est installé dans plusieurs formats (par exemple en CLAP et VST3), la bibliothèque d’appareils le liste une fois par format, chacun avec une pastille de format. Quand la liste contient au moins deux formats, des interrupteurs pour chaque format apparaissent en haut de la bibliothèque. Désactivez les formats que vous n’utilisez pas pour les masquer, dans la liste et dans sa recherche. Au moins un format reste toujours activé, et Ethereal retient votre choix.

## ouvrir la fenêtre d’un plug-in {#opening-a-plugins-window}

Cliquez sur le bouton de fenêtre dans l’en-tête d’un plug-in pour ouvrir l’interface propre au plug-in. Elle s’ouvre dans une fenêtre séparée qui reste au-dessus de la fenêtre principale d’Ethereal. Cliquez à nouveau sur le bouton pour la fermer. La carte du plug-in dans Ethereal liste aussi ses paramètres, avec le texte de valeur propre au plug-in (comme « 250 ms » ou « -6 dB »).

Sur Windows et Linux, une fenêtre de plug-in CLAP ne s’ouvre que si le plug-in sait ouvrir sa propre fenêtre. Beaucoup de plug-ins CLAP ne savent que s’afficher dans une fenêtre créée par l’hôte, ce qu’Ethereal ne fait pour l’instant que sur macOS : leur fenêtre ne s’ouvre pas, mais leurs paramètres restent sur leur carte. Les fenêtres VST3 s’ouvrent sur tous les systèmes.

### fenêtres de plug-ins sous linux {#plugin-windows-on-linux}

Sous Linux, les fenêtres des plug-ins ont besoin d’un affichage X. Sur un bureau Wayland, cela veut dire XWayland.

### touches dans les fenêtres de plug-ins {#keys-in-plugin-windows}

Quand une fenêtre de plug-in a le focus :

- [[action:transport.play]] lance et arrête toujours la lecture d’Ethereal, et les autres raccourcis de lecture fonctionnent toujours.
- Quand le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) est activé, ses touches de notes jouent la piste d’Ethereal au lieu d’aller au plug-in.
- Les combinaisons avec [[key:Ctrl]], [[key:Alt]] ou [[key:Mod]] vont toujours au plug-in, pour que ses propres raccourcis continuent de fonctionner.

## quand un plug-in plante {#when-a-plugin-crashes}

Par défaut, les plug-ins tournent à l’intérieur d’Ethereal. Pour un plug-in dont vous vous méfiez, faites un clic droit sur son en-tête et choisissez **exécuter isolé (protégé des plantages, +1 bloc de latence)**. Le plug-in tourne alors dans son propre processus. S’il plante, Ethereal continue de jouer : l’appareil affiche **planté · contourné** et la piste joue sans lui. Cliquez sur **⟳** pour recharger le plug-in depuis son dernier état enregistré.

L’isolation ajoute un bloc audio de latence. Pour revenir en arrière, faites un clic droit sur l’en-tête et choisissez **exécuter dans le processus (sans isolation)**.

> [!note]
> L’isolation fonctionne sur macOS et Linux. Sous Windows, les plug-ins tournent toujours à l’intérieur d’Ethereal : choisir d’exécuter un plug-in isolé n’y change rien.

## ouvrir un projet sans les plug-ins {#opening-a-project-without-plugins}

Si un projet plante ou se bloque à cause d’un plug-in, ouvrez-le sans les plug-ins. Faites un clic droit sur le projet dans la liste des projets (ou cliquez sur son bouton **…**) et choisissez **ouvrir sans plug-ins**. La fenêtre de récupération après plantage propose le même bouton. Chaque plug-in est gardé comme un emplacement contourné avec ses réglages enregistrés, marqué **mode sans échec · contourné**, et la barre du haut indique **plug-ins désactivés (mode sans échec)**. Ensuite :

- cliquez sur **⟳** sur un appareil pour charger seulement ce plug-in, ou
- cliquez sur **charger les plug-ins** dans la barre du haut pour les charger tous.

Voir [le mode sans échec](/docs/advanced/safe-mode) pour en savoir plus.

## plug-ins manquants {#missing-plugins}

Si un projet utilise un plug-in qui n’est pas installé sur cet ordinateur, Ethereal garde l’appareil et son son enregistré. Il est marqué **manquant · contourné** et la piste joue sans lui. Rien n’est perdu :

1. Installez le plug-in, puis **réanalyser** dans **réglages › plug-ins**.
2. L’appareil affiche maintenant **trouvé · recharger**. Cliquez dessus pour charger le plug-in avec le son enregistré dans le projet.

Pour utiliser un autre plug-in à la place, faites un clic droit sur le badge **manquant · contourné** ou sur l’en-tête de l’appareil et choisissez **résoudre…**. Faites un clic droit sur l’en-tête de n’importe quel plug-in et choisissez **remplacer le plug-in…** pour l’échanger contre un autre plug-in installé. Les valeurs et l’automation sont reportées sur les paramètres de même nom. Voir [plug-ins et échantillons manquants](/docs/import-export/other-daws#resolving-missing-items).

## comment le son des plug-ins est enregistré {#how-plugin-sounds-are-saved}

L’état complet de chaque plug-in est enregistré dans le fichier du projet. Ethereal note de quel plug-in il s’agit (son format et son identifiant), et non l’endroit où il est installé, donc le projet s’ouvre sur tout ordinateur qui a le plug-in. Quand vous quittez, Ethereal enregistre l’état actuel des plug-ins avec le projet.

## presets de plug-ins {#plugin-presets}

Le menu de presets dans l’en-tête d’un appareil fonctionne aussi pour les plug-ins : il enregistre et charge l’état complet du plug-in. Les presets de plug-ins sont stockés par plug-in dans votre bibliothèque utilisateur, à côté de vos autres presets. En charger un est une seule étape d’annulation. Voir [appareils et presets](/docs/making-music/devices-and-presets).

## et ensuite {#where-next}

- [Appareils et presets](/docs/making-music/devices-and-presets) : ajoutez et organisez des appareils sur une piste.
- [Le mode sans échec](/docs/advanced/safe-mode) : ouvrez un projet qui plante.
- [Projets d’autres DAW](/docs/import-export/other-daws) : les plug-ins dans les projets importés.
