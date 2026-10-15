---
title: ethereal sur votre téléphone
order: 10
summary: Activez la bêta mobile, repérez-vous dans la disposition tactile et sachez ce qui demande encore un ordinateur.
platform: mobile
audience: everyone
covers:
  - mobile/phones-show-a-not-available-screen #phones-and-tablets
  - mobile/turning-on-the-mobile-beta #turning-on-the-mobile-beta
  - mobile/the-mobile-layout #the-layout
  - mobile/what-s-missing-on-mobile #what-still-needs-a-computer
  - mobile/projects-sharing-and-settings-on-a-phone #projects-sharing-and-settings
  - mobile/leaving-the-app-stops-playback #leaving-the-app-stops-playback
  - mobile/rotating-keeps-your-place #rotating-the-phone
---

Ethereal est conçu pour un ordinateur avec une souris et un clavier. Une version tactile pour téléphones et tablettes est en développement : vous pouvez l’essayer dès maintenant en bêta dans le navigateur de votre téléphone. Cette page explique comment l’activer, ce que vous voyez et ce qui demande encore un ordinateur. Les gestes eux-mêmes sont décrits dans [gestes tactiles](/docs/mobile/gestures).

## téléphones et tablettes {#phones-and-tablets}

Quand vous ouvrez Ethereal dans le navigateur d’un téléphone, il ne lance pas l’éditeur. Il affiche à la place **ethereal n’est pas disponible sur mobile — essayez-le sur ordinateur**, avec trois possibilités :

- **copier le lien** : copie l’adresse, pour l’ouvrir plus tard sur un ordinateur.
- **ouvrez ce lien sur un ordinateur** : un rappel que l’éditeur complet tourne là-bas.
- **essayer la bêta mobile** : lance la version tactile (voir plus bas).

Pour décider, Ethereal regarde l’appareil, pas la taille de la fenêtre :

- Les téléphones sont bloqués dans les deux orientations, même quand le navigateur demande la version pour ordinateur.
- Les tablettes sans souris ni pavé tactile sont bloquées aussi, car rien ne permet de faire glisser avec précision.
- Une tablette avec un clavier et un pavé tactile (un iPad avec un Magic Keyboard, par exemple) a l’éditeur de bureau normal.
- Les ordinateurs portables à écran tactile ont l’éditeur de bureau.

Deux types de liens fonctionnent toujours sur un téléphone, avec ou sans la bêta : les [liens de partage statiques](/docs/sharing/static-links) (`/s/…`), qui ouvrent la [visionneuse de partage](/docs/sharing/share-viewer) en lecture seule, et cette documentation.

## activer la bêta mobile {#turning-on-the-mobile-beta}

Touchez **essayer la bêta mobile** sur l’écran « pas disponible », ou ajoutez vous-même `?mobile=1` à l’adresse. La version tactile démarre alors, avec le vrai moteur audio qui tourne dans votre navigateur.

- Le choix est mémorisé pour cet onglet du navigateur, donc recharger garde la bêta activée.
- `?mobile=0` dans l’adresse la désactive à nouveau pour l’onglet.
- `?force-desktop` lance quand même l’éditeur de bureau. Il est difficile à utiliser sans souris, mais il est là si vous avez besoin d’une fonction que la bêta n’a pas.

> [!warning]
> Sur le web, vos projets ne vivent que dans le stockage de ce navigateur. Effacer les données de votre navigateur les supprime. Exportez un projet (menu du projet → **exporter le projet…**) pour en garder une copie. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live).

## la disposition {#the-layout}

La bêta utilise le même arrangement que sur ordinateur, disposé pour le pouce :

![La bêta mobile sur un téléphone : la barre du haut, l’arrangement avec des clips, et le transport en bas](images/phone.jpg "La bêta mobile sur un téléphone.")

- **La barre du haut** : le menu du projet (touchez le nom du projet ; il contient aussi **exporter l’audio…**), **annuler** et **rétablir**, **partager** et le bouton des réglages.
- **L’arrangement** : les pistes et leurs clips. Sur un téléphone, chaque en-tête de piste affiche le nom puis **muet**, **solo** et **armer** en taille tactile. Le volume, le panoramique, les prises et l’automation sont dans la [fiche de piste](/docs/mobile/gestures#the-track-sheet).
- **La ligne d’état** : la position, le tempo et la signature rythmique, et le bouton **suivre la tête de lecture**.
- **Le transport** en bas : **arrêter**, **lecture** / **pause**, **enregistrer**, **boucle**, **métronome**, et un bouton qui bascule entre les pistes et le [mixeur](/docs/mobile/editing-on-touch#the-mixer).

Quand vous tournez un téléphone à l’horizontale, la barre du haut et le transport passent sur une seule ligne en bas, pour que l’arrangement ait toute la hauteur. Sur une tablette, l’état et le transport tiennent sur une ligne en bas dans les deux orientations, et les en-têtes de piste gardent leur volume et leur panoramique.

Il n’y a pas de barre latérale, pas de navigateur de fichiers et pas de fenêtres d’appareils flottantes. Les panneaux s’ouvrent comme des fiches qui glissent depuis le bas.

## ce qui demande encore un ordinateur {#what-still-needs-a-computer}

La bêta sert à arranger et modifier ce qui est déjà dans un projet. Ces choses ne sont pas encore sur mobile :

- **Ajouter des sons** : il n’y a ni navigateur d’échantillons ni moyen de déposer des fichiers, vous ne pouvez donc pas importer d’audio. Les nouveaux clips sont des clips MIDI : touchez deux fois le couloir d’une piste MIDI pour en créer un.
- **Les appareils** : la fiche de piste liste les appareils d’une piste, mais vous ne pouvez pas les modifier ni ouvrir leurs fenêtres (« pour l’instant, modifiez les appareils sur un ordinateur. »). Pas de plug-ins non plus.
- **L’enregistrement** : le navigateur n’a pas d’entrées audio ou MIDI pour Ethereal, l’enregistrement demande donc l’application de bureau. Le bouton **enregistrer** est grisé. Voir [l’enregistrement](/docs/making-music/recording).
- **Les fonctions clavier** : pas de palette de commandes, de raccourcis clavier ni de clavier MIDI d’ordinateur.
- L’**assignation MIDI**, les visites guidées et l’assistant IA.

Pour signaler un problème depuis votre téléphone, utilisez **signaler un problème…** dans le menu du projet (il n’y a pas de barre d’état sur mobile).

## projets, partage et réglages {#projects-sharing-and-settings}

L’écran des projets, le menu du projet, le partage et les réglages fonctionnent tous sur un téléphone :

- Les fenêtres comme les réglages s’ouvrent en plein écran, et leurs boutons sont à taille tactile.
- Sur l’écran des projets, les boutons **renommer**, **dupliquer** et **supprimer** de chaque projet sont toujours affichés (il n’y a pas de survol sur un écran tactile).
- L’adresse du projet est conservée dans la barre d’adresse, donc recharger la page rouvre le même projet.
- **partager** fonctionne comme sur un ordinateur. Voir [partager votre musique](/docs/sharing/overview).
- Un lien d’invitation à une session en direct affiche d’abord l’écran « pas disponible » sur un téléphone. Touchez-y **essayer la bêta mobile** (ou ouvrez le lien dans un onglet où la bêta est déjà activée) : l’écran pour rejoindre apparaît, et **continuer dans le navigateur** rejoint la session dans la version tactile. Voir [rejoindre une session](/docs/sharing/joining).

## quitter l’application arrête la lecture {#leaving-the-app-stops-playback}

Quand vous quittez le navigateur (changement d’application, verrouillage de l’écran, appel), la lecture s’arrête. Si un enregistrement était en cours, cette prise est terminée. Revenir ne relance pas la lecture : touchez **lecture** quand vous êtes prêt.

## tourner le téléphone {#rotating-the-phone}

Tourner le téléphone garde votre position : le temps au milieu de l’écran reste au milieu, et le zoom et la sélection sont conservés. La disposition bascule entre portrait et paysage comme décrit dans [la disposition](#the-layout).

## et ensuite {#where-next}

- [Gestes tactiles](/docs/mobile/gestures) : faire défiler, zoomer, sélectionner et déplacer des clips.
- [Modifier au toucher](/docs/mobile/editing-on-touch) : le piano roll, l’automation et le mixeur.
- [Où vivent vos projets](/docs/welcome/where-your-projects-live).
