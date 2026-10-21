---
title: le navigateur
order: 10
summary: Trouvez, pré-écoutez et faites glisser des échantillons, chargez des presets et ajoutez vos propres dossiers d’échantillons.
platform: all
audience: beginner
covers:
  - browser/the-browser #finding-sounds
  - browser/preview-samples #previewing-samples
  - browser/drag-samples-into-the-project #dragging-samples-in
  - browser/midi-files-from-the-browser #midi-files
  - browser/presets-in-the-browser #presets
  - browser/add-your-own-folders #adding-your-own-folders
  - browser/folder-depth-limit #what-gets-indexed
  - browser/tags-and-favourites #favourites-and-tags
  - browser/packs #packs
  - browser/file-filter-rules #choosing-which-files-are-listed
  - browser/project-media-and-open-projects #projects
---

Le navigateur est l’endroit où vivent vos sons : échantillons, fichiers MIDI, presets d’appareils et projets. Cette page montre comment trouver un son, l’écouter et le placer dans votre morceau. Il vous faut un projet ouvert. Si vous n’en avez pas encore, commencez par [votre premier rythme](/docs/welcome/first-beat).

## ouvrir le navigateur {#opening-the-browser}

Cliquez sur l’icône **bibliothèque** en haut de la barre d’icônes, sur le bord gauche de la fenêtre. Un nouveau clic ferme le panneau. Vous pouvez aussi l’ouvrir depuis la palette de commandes ([[action:palette.open]]) avec **ouvrir la bibliothèque**.

## trouver des sons {#finding-sounds}

Les pastilles sous le champ de recherche choisissent ce que vous parcourez :

- **échantillons** : les fichiers audio.
- **midi** : les fichiers MIDI.
- **presets** : les réglages d’appareils enregistrés, ceux d’usine comme les vôtres.
- **projets** : vos projets Ethereal.

Sans recherche, **échantillons** et **midi** affichent votre bibliothèque sous forme d’arborescence de dossiers. Cliquez sur un dossier pour l’ouvrir sur place. Quand vous tapez dans le champ de recherche, Ethereal cherche dans toute la bibliothèque à la fois. Les dossiers dont le nom correspond viennent en premier, puis les fichiers. Cliquez sur un dossier trouvé pour l’afficher dans l’arborescence.

À côté du champ de recherche, vous pouvez trier les résultats par **nom**, **récents**, **durée** ou **bpm**. Pendant la saisie, les résultats sont triés par **pertinence**. L’arborescence est toujours triée par nom. Les lignes indiquent le tempo, la tonalité et la durée d’un échantillon quand Ethereal a pu les lire. La ligne d’état en bas compte les résultats et montre la progression de l’indexation pendant la lecture de nouveaux dossiers.

## pré-écouter des échantillons {#previewing-samples}

Cliquez sur un échantillon pour l’écouter, et cliquez à nouveau pour l’arrêter. Depuis le champ de recherche, [[key:ArrowUp]] et [[key:ArrowDown]] parcourent les résultats et jouent chaque échantillon au passage. [[key:Enter]] lance ou arrête celui qui est en surbrillance, et [[key:Escape]] efface la recherche.

Activez **sync** en bas du navigateur pour pré-écouter les échantillons calés sur le tempo du projet. Ethereal retient ce choix.

## faire glisser des échantillons {#dragging-samples-in}

Faites glisser un échantillon du navigateur vers l’arrangement :

- **sur une piste audio** : un clip audio est créé là où vous le déposez, aligné sur la grille. Maintenez [[key:Alt]] en le déposant pour le placer sans alignement.
- **dans l’espace vide sous les pistes** : une nouvelle piste audio est créée avec le clip dessus.

Dans les deux cas, l’import, la nouvelle piste et le clip forment une seule étape d’annulation. Pendant la lecture du fichier, un emplacement provisoire montre sa progression dans le couloir. Les échantillons ne peuvent pas être déposés sur des pistes MIDI.

Vous pouvez aussi faire glisser des échantillons directement sur les appareils qui jouent des fichiers audio : un **sampler**, un pad d’un [drum rack](/docs/making-music/drum-rack), une touche d’un **multisampler**, ou la réponse impulsionnelle d’une **réverb à convolution**.

Les fichiers de votre ordinateur peuvent aussi être déposés sur l’arrangement. Voir [importer de l’audio](/docs/import-export/importing-audio).

## fichiers midi {#midi-files}

Les fichiers MIDI sont listés sous la pastille **midi**, où vous pouvez les chercher, les marquer d’une étoile et les étiqueter. Faites-en glisser un sur une piste MIDI, ou sous les pistes, pour le transformer en clips MIDI. Voir [fichiers midi](/docs/import-export/midi-files).

## presets {#presets}

La pastille **presets** liste les presets d’usine et ceux que vous avez enregistrés. Pour charger un preset :

1. Sélectionnez une piste qui contient un appareil du type du preset (par exemple un **synthé** pour un preset de synthé).
2. Double-cliquez sur le preset, ou appuyez sur [[key:Enter]] dessus, ou faites un clic droit et choisissez **charger sur …**.

Si la piste sélectionnée n’a pas d’appareil correspondant, le navigateur vous demande d’en sélectionner une qui en a un. Les presets ne peuvent pas être glissés sur des pistes. Pour enregistrer des presets, utilisez le menu de presets de l’appareil (voir [appareils et presets](/docs/making-music/devices-and-presets)).

## projets {#projects}

La pastille **projets** liste vos projets Ethereal. Double-cliquez sur l’un d’eux, ou faites un clic droit et choisissez **ouvrir un projet**, pour l’ouvrir.

## ajouter vos propres dossiers {#adding-your-own-folders}

La dernière ligne de l’arborescence ajoute vos propres dossiers d’échantillons à la bibliothèque :

- **ajouter un dossier…** [[desktop]] : choisissez un dossier sur votre ordinateur. Ethereal l’indexe là où il est et ne copie pas les fichiers. Si vous déplacez ou supprimez le dossier, ses échantillons disparaissent du navigateur.
- **importer un dossier…** [[web]] : choisissez un dossier, et Ethereal copie ses fichiers audio dans la bibliothèque stockée dans votre navigateur. La progression s’affiche en bas du navigateur.

Vous pouvez aussi déposer un dossier de votre ordinateur sur le navigateur. Sur l’application de bureau, il est ajouté sur place ; sur le web, il est copié. Les fichiers audio déposés sur le navigateur (et non les dossiers) sont ajoutés au projet ouvert sans créer de clip.

Faites un clic droit sur l’un de vos dossiers dans l’arborescence pour :

- **réanalyser** : relire le dossier après avoir ajouté ou retiré des fichiers.
- **renommer…** : changer le nom affiché dans le navigateur. Laissez le champ vide pour revenir au nom du dossier lui-même.
- **retirer le dossier** : le sortir de la bibliothèque. Vos fichiers restent sur le disque.

> [!warning]
> Sur le web, la bibliothèque vit dans le stockage de votre navigateur, comme vos projets. Effacer les données du site la supprime. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live).

### ce qui est indexé {#what-gets-indexed}

Ethereal lit les sous-dossiers jusqu’à 16 niveaux de profondeur. Les fichiers et dossiers cachés (dont le nom commence par un point, comme `.DS_Store` ou `.git`) sont ignorés.

## packs {#packs}

Un pack d’échantillons placé dans l’un de vos dossiers apparaît comme un dossier à part, sous le dossier qui le contient. Un pack garde son nom exactement tel qu’il est écrit.

## favoris et tags {#favourites-and-tags}

Cliquez sur l’étoile d’une ligne pour ajouter l’élément à vos favoris. L’étoile à côté des pastilles affiche **favoris uniquement**.

Faites un clic droit sur un élément pour plus d’options :

- **ajouter aux favoris** / **retirer des favoris**.
- **modifier les tags…** : tapez des tags séparés par des virgules, par exemple `drums, punchy, dark`.
- **afficher dans le dossier** (échantillons et MIDI) : aller au dossier de l’élément dans l’arborescence.
- **filtrer par « … »** : n’afficher que les éléments qui ont ce tag.

Les tags s’affichent aussi sur les lignes. Cliquez sur l’un d’eux pour filtrer par ce tag. Les filtres de tags actifs se trouvent sous les pastilles, et un clic sur l’un d’eux le retire.

## choisir les fichiers listés {#choosing-which-files-are-listed}

Par défaut, le navigateur ne liste que les fichiers qu’Ethereal peut utiliser, et masque les fichiers système et annexes. Pour changer cela, ouvrez les **réglages**, allez dans **général** › **bibliothèque** et trouvez **fichiers de la bibliothèque** :

- **afficher** : les règles des fichiers à lister. Sans règle, tous les fichiers sont listés.
- **masquer** : les règles des fichiers et dossiers à exclure. Une règle de masquage l’emporte sur une règle d’affichage.

Les règles sont des expressions régulières appliquées aux noms de fichiers, sans tenir compte de la casse. Par exemple, `\.wav$` correspond aux fichiers WAV. Une règle invalide est signalée pendant la saisie, et rien n’est enregistré tant que toutes les règles ne sont pas valides. La ligne sous les listes indique combien de fichiers de votre bibliothèque les règles afficheraient. **rétablir les réglages par défaut** remet les règles d’origine.

Les projets de DAW (`.als`, `.flp`, `.rpp`, `.dawproject`) ne s’ouvrent pas depuis le navigateur.

## et ensuite {#where-next}

- [Les plug-ins](/docs/sounds-and-plugins/plugins) : utilisez vos propres instruments et effets.
- [Importer de l’audio](/docs/import-export/importing-audio) : apportez des fichiers audio de votre ordinateur.
- [Les clips](/docs/making-music/clips) : déplacez, rognez et bouclez ce que vous avez glissé.
