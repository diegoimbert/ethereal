---
title: votre première boucle
order: 30
summary: Faites une courte boucle en cinq minutes : des notes, un deuxième instrument, un effet et la lecture.
platform: all
audience: beginner
covers:
---

Ce pas-à-pas vous mène d’un projet vide à une courte boucle de deux instruments avec un effet. Il n’utilise que ce qui est fourni avec Ethereal, dans l’application de bureau ou la version web. Chaque étape renvoie à la page qui l’explique plus en détail.

## 1. créer un projet {#1-start-a-project}

Au démarrage, Ethereal affiche l’écran d’accueil. Cliquez sur **nouveau projet**, ou fermez l’écran d’accueil avec [[key:Escape]] : dans les deux cas, vous obtenez un nouveau projet avec une piste MIDI appelée **synthé**, sur laquelle se trouve le synthé intégré.

Inutile d’enregistrer : Ethereal enregistre au fur et à mesure (voir [projets et enregistrement](/docs/basics/projects)).

## 2. créer un clip {#2-make-a-clip}

Un clip contient des notes. Double-cliquez dans le couloir vide de la piste **synthé**, près du début : un clip d’une mesure apparaît. Cliquez sur le clip pour l’ouvrir dans le piano roll, en bas de la fenêtre.

## 3. dessiner des notes {#3-draw-some-notes}

Dans le piano roll, les lignes sont les hauteurs (le clavier à gauche) et les colonnes le temps.

- **Double-cliquez** sur un emplacement vide pour ajouter une note. Gardez le bouton enfoncé après le second clic et faites glisser vers la droite pour l’allonger.
- Faites glisser une note pour la déplacer, faites glisser son bord droit pour changer sa durée, et double-cliquez dessus pour la supprimer.
- Appuyez sur [[action:pianoRoll.drawMode]] pour passer en mode dessin, où chaque clic ajoute une note. Appuyez de nouveau pour revenir.

Dessinez quatre ou cinq notes. Vous entendez chaque note au moment où vous l’ajoutez. Plus de détails dans [le piano roll](/docs/making-music/piano-roll).

> [!tip]
> Pas d’idée ? Activez le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) avec [[action:midi.computerKeyboard]] et jouez des notes avec les touches de lettres.

## 4. l’écouter {#4-play-it}

Appuyez sur [[action:transport.play]] pour lancer la lecture, et de nouveau pour arrêter et revenir là où vous aviez commencé. Pour entendre la mesure en boucle, activez le bouton **boucle** dans la barre du haut : la lecture répète la zone de l’accolade de boucle au-dessus des pistes, que vous pouvez faire glisser pour couvrir votre clip. Voir [lire et se déplacer](/docs/basics/playing-and-navigating).

## 5. ajouter un deuxième instrument {#5-add-a-second-instrument}

Cliquez sur **ajouter une piste** au-dessus des en-têtes de piste et choisissez **piste MIDI**. La bibliothèque d’instruments s’ouvre sous la nouvelle piste : choisissez un autre instrument, par exemple le **synthé poly**, ou cliquez ailleurs pour garder le synthé par défaut. Créez aussi un clip sur cette piste et dessinez une ligne de basse ou quelques accords, plus bas. Voir [les pistes](/docs/making-music/tracks) et [les instruments](/docs/making-music/instruments).

Chaque instrument a des presets d’usine : sélectionnez la piste, puis dans l’**inspecteur** à droite, survolez la carte de l’instrument et ouvrez **presets**. Essayez-en quelques-uns.

## 6. ajouter un effet {#6-add-an-effect}

Quand une piste est sélectionnée, l’inspecteur affiche ses appareils. Cliquez sur **ajouter un effet audio** sous l’instrument et choisissez **réverbération**. Sa fenêtre s’ouvre : tournez un bouton et écoutez. Voir [les effets](/docs/making-music/effects).

## 7. équilibrer le tout {#7-balance-it}

Chaque en-tête de piste a un fader de volume et un bouton de panoramique. Baissez ce qui est trop fort. Si vous faites une erreur, où que ce soit, [[action:edit.undo]] l’annule.

## et ensuite {#where-next}

Vous avez fait une boucle. Pour la suite :

- [Une visite de l’écran](/docs/basics/the-screen) : à quoi sert chaque partie de la fenêtre.
- [Les clips](/docs/making-music/clips) : copiez, bouclez et arrangez vos clips pour en faire un morceau.
- [Le navigateur](/docs/sounds-and-plugins/browser) : ajoutez vos propres échantillons, et construisez un beat avec le [drum rack](/docs/making-music/drum-rack).
- [Exporter l’audio](/docs/import-export/exporting-audio) : transformez votre morceau en fichier WAV ou MP3 à partager.
