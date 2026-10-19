---
title: fichiers midi
order: 15
summary: Transformez des fichiers MIDI en clips MIDI, et enregistrez vos clips MIDI en fichiers MIDI pour d’autres applications.
platform: all
audience: everyone
covers:
---

Un fichier MIDI (`.mid` ou `.midi`) contient des notes, pas du son : une mélodie, un motif de batterie ou toutes les parties d’un morceau, créés dans une autre application ou téléchargés. Cette page montre comment en faire entrer un dans un projet sous forme de clips MIDI, et comment enregistrer vos propres clips en fichier MIDI. Il vous faut un projet ouvert.

## importer un fichier midi {#importing-a-midi-file}

Faites glisser un fichier MIDI sur l’arrangement, depuis [le navigateur](/docs/sounds-and-plugins/browser#midi-files) ou depuis le gestionnaire de fichiers de votre ordinateur :

- **sur une piste MIDI** : la première partie du fichier devient un clip sur cette piste, là où vous le déposez. Chaque autre partie du fichier reçoit une nouvelle piste MIDI juste après.
- **sous les pistes** : chaque partie du fichier reçoit une nouvelle piste MIDI, toutes à partir de l’endroit où vous le déposez.
- **sur une piste audio** : un fichier venant du navigateur ne peut pas y aller. Un fichier venant de votre ordinateur va sur de nouvelles pistes MIDI, comme s’il était déposé sous les pistes.

Chaque partie devient un clip. Ses notes gardent leur vélocité et leur durée, et le clip dure aussi longtemps que tout le fichier, arrondi à la mesure entière supérieure. Les nouvelles pistes reçoivent le **synthé** intégré, et portent le nom de la partie dans le fichier (ou celui du fichier et de son canal MIDI quand la partie n’a pas de nom). Un fichier qui garde toutes ses parties dans une seule piste (un fichier « type 0 ») est séparé par canal MIDI : la batterie et la basse ont donc quand même leurs propres pistes.

Tout l’import compte comme une seule étape d’annulation : [[action:edit.undo]] retire toutes les pistes et tous les clips qu’il a ajoutés. Les nouveaux clips sont sélectionnés, prêts à être déplacés.

### tempo {#tempo}

Les notes sont placées en temps, elles restent donc sur la grille. Dans un projet qui n’a encore aucun clip, le tempo et la signature rythmique du fichier (et leurs changements) deviennent ceux du projet : le morceau joue à la vitesse à laquelle il a été écrit. Dans un projet qui a déjà des clips, le projet garde son tempo et les notes le suivent.

## exporter du midi {#exporting-midi}

Pour enregistrer des clips MIDI en fichier MIDI standard, que n’importe quelle DAW ou application de partition sait ouvrir :

- **un clip, ou plusieurs** : sélectionnez des clips MIDI, faites un clic droit sur l’un d’eux et choisissez **exporter en MIDI…**. Le fichier commence au premier clip sélectionné.
- **une piste entière** : faites un clic droit sur l’en-tête d’une piste MIDI et choisissez **exporter en MIDI…**. Tous les clips MIDI de la piste y entrent, et le fichier commence au début du projet : les clips gardent donc leur place. Avec plusieurs pistes sélectionnées, chacune devient une partie du fichier. Sur une piste sans clip MIDI, l’entrée est grisée.

Les clips sont écrits tels qu’ils jouent : un clip en boucle est répété sur toute sa longueur, et un clip rogné ne donne que les notes que vous entendez. Les notes muettes sont exclues. Le fichier contient aussi le tempo et les signatures rythmiques du projet.

Sur le bureau, une fenêtre d’enregistrement vous demande où placer le fichier. Dans le navigateur, le fichier est téléchargé.

## et ensuite {#where-next}

- [Le piano roll](/docs/making-music/piano-roll) : modifiez les notes importées.
- [Importer de l’audio](/docs/import-export/importing-audio) : importez des fichiers audio de la même façon.
