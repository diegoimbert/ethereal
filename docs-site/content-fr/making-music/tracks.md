---
title: pistes
order: 10
summary: Ajoutez, sélectionnez, mettez en solo, renommez, réordonnez, redimensionnez et groupez des pistes, et découvrez à quoi sert chaque type de piste.
platform: all
audience: beginner
covers:
  - arrangement/adding-tracks-menu-drag-or-empty-space-right-click #adding-a-track
  - arrangement/new-midi-track-opens-the-instrument-picker #choosing-an-instrument
  - arrangement/track-types #kinds-of-tracks
  - arrangement/selecting-tracks-click-shift-cmd-ctrl #selecting-tracks
  - arrangement/solo-is-exclusive-cmd-ctrl-click-adds #mute-and-solo
  - arrangement/arm-is-exclusive-cmd-ctrl-click-to-arm-several #arming-for-recording
  - arrangement/header-volume-and-pan #volume-and-pan
  - arrangement/rename-tracks-empty-name-restores-the-default #renaming-a-track
  - arrangement/reorder-tracks-by-dragging-headers #moving-tracks
  - arrangement/track-height-header-width #track-height-and-header-width
  - arrangement/group-and-ungroup-tracks #groups
  - arrangement/vca-tracks #vca-tracks
  - arrangement/track-menu-cut-copy-paste-duplicate-delete-tracks #cut-copy-paste-duplicate-and-delete
  - arrangement/cut-tracks-can-only-be-pasted-right-away #cut-copy-paste-duplicate-and-delete
  - arrangement/track-input-from-another-track #recording-from-another-track
---

Les pistes sont les rangées de l’arrangement. Chacune contient des clips (ou d’autres pistes, pour un groupe) et possède sa propre chaîne d’appareils. Cette page explique comment les ajouter et les organiser. Il vous suffit d’un projet ouvert.

## ajouter une piste {#adding-a-track}

Il y a trois façons d’ajouter une piste :

- Cliquez sur **ajouter une piste** dans le coin au-dessus des en-têtes de piste, ou sur la rangée **ajouter une piste** sous la dernière piste. Le menu propose une **piste midi**, une **piste audio** ou une **piste de retour**. Un clic l’ajoute après la dernière piste (une piste de retour juste après le retour sélectionné, ou après les autres retours, au-dessus du master).
- Faites glisser une entrée hors de ce menu et déposez-la entre deux pistes, ou sur l’en-tête d’un groupe, pour la créer à cet endroit. Une piste de retour ne peut être déposée qu’entre les autres retours.
- Faites un clic droit dans l’espace vide sous les pistes et choisissez **ajouter une piste audio**, **ajouter une piste midi** ou **ajouter une piste de retour**. Le même menu contient **importer de l’audio…**. Le menu contextuel d’une piste propose aussi **ajouter une piste de retour**.

La palette de commandes ([[action:palette.open]]) a aussi des commandes pour ajouter une piste audio, MIDI ou de retour.

### choisir un instrument {#choosing-an-instrument}

Une nouvelle piste MIDI ouvre une bibliothèque d’instruments sous son en-tête. Choisissez un instrument pour remplacer le synthé par défaut de la piste, ou cliquez ailleurs pour garder le synthé. Vous pourrez changer d’instrument plus tard dans les appareils de la piste (voir [appareils et presets](/docs/making-music/devices-and-presets)).

## types de pistes {#kinds-of-tracks}

| type | ce qu’elle contient |
|---|---|
| **midi** | Des clips MIDI (des notes), joués par l’instrument placé au début de sa chaîne d’appareils. |
| **audio** | Des clips audio (enregistrements et échantillons). |
| **groupe** | D’autres pistes. Leur son est additionné dans le groupe, si bien qu’un seul fader et une seule chaîne d’appareils peuvent tous les traiter. |
| **vca** | Rien à jouer. Elle monte ou baisse le volume des pistes qui lui sont assignées. |
| **retour** | Des effets que les pistes atteignent par des envois. Les retours sont en bas, au-dessus du master. |
| **master** | Le mixage final. Elle est fixée en bas de l’arrangement. |

Un groupe peut contenir des pistes audio, MIDI et de groupe, mais pas de retours ni de VCA.

> [!note]
> Le menu **ajouter une piste** crée des pistes MIDI, audio et de retour. Les groupes et les VCA viennent des commandes ci-dessous. Voir [envois et pistes de retour](/docs/making-music/mixing#sends-and-return-tracks) pour savoir à quoi sert une piste de retour.

## sélectionner des pistes {#selecting-tracks}

Cliquez sur l’en-tête d’une piste pour la sélectionner. L’inspecteur affiche alors ses réglages. Un [[key:Shift]]-clic sélectionne une plage de pistes, et un [[key:Mod]]-clic ajoute une piste à la sélection ou l’en retire. Cliquer dans l’espace vide du couloir d’une piste sélectionne aussi cette piste.

## muet et solo {#mute-and-solo}

Chaque en-tête a un bouton muet (le haut-parleur) et un bouton solo (le casque).

Le solo est exclusif. Un simple clic met seulement cette piste en solo et retire le solo des autres. Cliquer sur la seule piste en solo retire son solo. Maintenez [[key:Mod]] en cliquant pour ajouter une piste aux pistes en solo, ou l’en retirer, sans toucher aux autres.

## armer pour l’enregistrement {#arming-for-recording}

Le bouton rond d’armement prépare une piste à l’enregistrement. L’armement est exclusif lui aussi : un simple clic arme seulement cette piste. Un [[key:Mod]]-clic en arme plusieurs. Si vous armez une piste audio alors qu’aucun périphérique d’entrée n’est ouvert, Ethereal propose d’en choisir un. Voir [enregistrement](/docs/making-music/recording).

## volume et panoramique {#volume-and-pan}

À l’extrémité droite de chaque en-tête se trouvent un petit fader de volume horizontal et un bouton de panoramique.

- **Volume** : faites glisser vers la gauche ou la droite. Maintenez [[key:Shift]] pour un réglage fin. Double-cliquez pour le remettre à 0 dB. La valeur en dB s’affiche pendant le survol ou le glissement.
- **Panoramique** : faites glisser vers le haut ou le bas. Double-cliquez pour le recentrer.

Si plusieurs pistes sont sélectionnées, faire glisser le fader de l’une d’elles les déplace toutes du même nombre de dB, si bien que leur équilibre reste le même.

## renommer une piste {#renaming-a-track}

Double-cliquez sur le nom d’une piste, tapez, puis appuyez sur [[key:Enter]] ou cliquez ailleurs. [[key:Escape]] annule. Si vous effacez le nom, la piste reprend son nom par défaut. Une piste MIDI sans nom affiche le nom de son instrument.

## déplacer des pistes {#moving-tracks}

Faites glisser un en-tête vers le haut ou le bas. Une ligne montre où ira la piste : relâchez entre deux rangées pour l’y placer. Relâchez au milieu de l’en-tête d’un groupe pour mettre la piste dans le groupe. Un groupe se déplace avec les pistes qu’il contient. Près du bord haut ou bas de l’arrangement, la vue défile. Appuyez sur [[key:Escape]] pour annuler le glissement. Chaque déplacement compte pour une étape d’annulation.

Les pistes de retour restent ensemble en bas, au-dessus du master, et vous pouvez les réordonner de la même façon : faites glisser l’en-tête d’un retour entre deux autres retours. Au-dessus des pistes ordinaires, celles-ci passent au rouge et le pointeur indique que le retour ne peut pas aller là. Les envois continuent d’aller vers le même retour, où qu’il aille. Le master ne bouge jamais.

## hauteur des pistes et largeur des en-têtes {#track-height-and-header-width}

- Faites glisser le bord inférieur de l’en-tête d’une piste pour l’agrandir ou la réduire. Elle change par paliers. Maintenez [[key:Alt]] pour redimensionner librement. Double-cliquez sur le bord pour la réinitialiser.
- Faites glisser le bord entre les en-têtes et les couloirs pour élargir ou rétrécir la colonne des en-têtes. Double-cliquez dessus pour la réinitialiser. Quand la colonne devient étroite, le sélecteur **grille** passe dans un menu **…** dans le coin.

## groupes {#groups}

Sélectionnez des pistes et appuyez sur [[action:track.group]] (ou faites un clic droit sur un en-tête et choisissez **grouper la piste** / **grouper 3 pistes**). Si les pistes viennent de groupes différents, elles sont d’abord rapprochées, puis réunies dans un nouveau groupe.

- Cliquez sur la pastille ronde d’un groupe (le chevron) pour le replier ou le déplier. Le couloir d’un groupe replié résume où ses pistes ont des clips.
- [[action:track.ungroup]] (ou **dégrouper** dans le menu de l’en-tête) supprime le groupe et laisse ses pistes en place. Si le groupe a ses propres appareils, automation, envois ou routages, Ethereal demande d’abord confirmation, car ils sont supprimés avec lui. Vous pouvez l’annuler.

## pistes vca {#vca-tracks}

Un VCA contrôle le volume d’autres pistes sans transporter d’audio. Sélectionnez les pistes, faites un clic droit sur l’un de leurs en-têtes et choisissez **nouveau vca pour la piste** (ou **nouveau vca pour 3 pistes**). Le même menu permet d’**assigner au VCA** ou de **retirer du VCA**. L’inspecteur a aussi une rangée **vca** pour chaque piste, et l’inspecteur d’un VCA liste les pistes qu’il contrôle.

## couper, copier, coller, dupliquer et supprimer {#cut-copy-paste-duplicate-and-delete}

Faites un clic droit sur un en-tête pour agir sur toutes les pistes sélectionnées. Les libellés indiquent combien (**supprimer 3 pistes**). Les raccourcis habituels fonctionnent aussi quand des pistes sont sélectionnées : [[action:edit.cut]], [[action:edit.copy]], [[action:edit.paste]], [[action:edit.duplicate]] et [[action:edit.delete]]. La piste master n’a pas de menu.

- **Dupliquer** copie tout : appareils, clips, automation et envois. Les copies apparaissent juste après les pistes sélectionnées.
- **Coller** place des copies des pistes copiées après la piste sélectionnée (ou à la fin).
- **Couper** supprime les pistes. Les coller les restaure seulement tant que la coupe est votre dernière étape d’annulation. Après toute autre modification, les pistes coupées ne peuvent plus être collées, mais [[action:edit.undo]] les fait toujours revenir.

Les copies de pistes de retour et de VCA restent juste après leur source, car un groupe ne peut pas les contenir.

## couleurs {#colors}

Chaque nouvelle piste reçoit une couleur de la palette. Pour la changer, sélectionnez la piste et choisissez une couleur dans l’inspecteur, à côté de son nom. Les clips prennent la couleur de leur piste, sauf si vous leur en donnez une (voir [clips](/docs/making-music/clips#clip-colors)).

## enregistrer depuis une autre piste {#recording-from-another-track}

L’entrée d’une piste audio peut être une autre piste au lieu d’une interface audio, pour enregistrer (rééchantillonner) ce que joue cette piste. Sélectionnez la piste audio et ouvrez la rangée **entrée** dans l’inspecteur : choisissez **depuis** et la piste source, puis l’endroit où prendre son signal :

- **pré-fx** : avant les appareils de la piste source.
- **post-fx** : après ses appareils.
- **post-mixeur** : après son volume et son panoramique.

Choisir une piste règle aussi le monitoring de la piste sur **in**, pour que vous entendiez la source à travers elle. Voir [enregistrement](/docs/making-music/recording).

## et ensuite {#where-next}

- [clips](/docs/making-music/clips) : créez, déplacez et modifiez ce qui se trouve sur les pistes.
- [appareils et presets](/docs/making-music/devices-and-presets) : les instruments et les effets d’une piste.
- [arrangement](/docs/making-music/arranging) : modifiez des sections entières du morceau d’un coup.
