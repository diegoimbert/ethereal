---
title: qu’est-ce qu’ethereal
order: 10
summary: Ce qu’est Ethereal, l’application de bureau, la version web et la bêta mobile, et comment recevoir les mises à jour.
platform: all
audience: beginner
covers:
  - first-run/what-the-web-version-can-t-do #what-the-web-version-cant-do
  - remote-engine/web-vs-desktop-plugins #what-the-web-version-cant-do
  - remote-engine/web-vs-desktop-recording-and-midi #what-the-web-version-cant-do
  - remote-engine/web-hosting-and-dedicated-engines #what-the-web-version-cant-do
  - remote-engine/web-phones-and-big-projects #phones-and-tablets
  - first-run/the-open-on-desktop-band #the-open-on-desktop-band
  - first-run/phones-and-touch-only-tablets-are-blocked #phones-and-tablets
  - first-run/desktop-updates #updates
  - first-run/linux-package-installs-don-t-update-themselves #updates
---

Ethereal est une application de musique (une STAN, station de travail audionumérique, ou DAW en anglais) : vous composez des morceaux avec des pistes de notes et d’audio, vous les jouez à travers des instruments et des effets, vous enregistrez, mixez et exportez. Vous pouvez aussi faire de la musique avec des amis, en direct, en partageant un lien.

Cette page présente les trois façons de l’utiliser et ce que chacune permet. Pour commencer tout de suite, allez à [votre première boucle](/docs/welcome/first-beat).

## trois façons de l’utiliser {#three-ways-to-run-it}

| | où | idéal pour |
|---|---|---|
| **l’application de bureau** [[desktop]] | macOS, Windows et Linux, depuis [la page de téléchargement](/download) | tout : plug-ins, enregistrement, apprentissage MIDI, votre interface audio |
| **la version web** [[web]] | un navigateur sur un ordinateur, sur ethereal.ws | essayer Ethereal, noter une idée, rejoindre la session d’un ami |
| **la bêta mobile** [[mobile]] | le navigateur d’un téléphone ou d’une tablette | écouter et faire de petites modifications en déplacement (en développement) |

Les trois sont la même application, avec le même format de projet et les mêmes instruments et effets intégrés. Le moteur qui produit le son tourne sur votre propre ordinateur dans tous les cas : en natif dans l’application de bureau, dans le navigateur pour la version web.

## ce que la version web ne peut pas faire {#what-the-web-version-cant-do}

Le navigateur limite ce qu’une page web peut faire de votre ordinateur, alors certaines fonctions demandent l’application de bureau. Quand vous en rencontrez une, l’application le dit (« … demande l’application de bureau ») :

- **Les plug-ins** (CLAP, VST3, AU) : la bibliothèque d’appareils indique « les plug-ins sont disponibles dans l’application de bureau ». Tous les instruments et effets intégrés fonctionnent. Voir [les plug-ins](/docs/sounds-and-plugins/plugins).
- **L’enregistrement** de l’audio ou du MIDI depuis vos entrées. Voir [l’enregistrement](/docs/making-music/recording).
- L’**apprentissage MIDI**, et le routage matériel avec la mesure de latence (instruments et effets externes).
- **Le choix de vos périphériques audio** : dans le navigateur, c’est votre système qui choisit la sortie. Réglages › audio l’indique.
- **Faire tourner un moteur pour les autres** dans une session en direct : seule l’application de bureau peut jouer pour quelqu’un qui n’a pas de moteur. Une session en direct que vous animez depuis le navigateur s’arrête aussi quand vous fermez l’onglet ou que l’ordinateur se met en veille.

> [!warning]
> Les projets web ne vivent que dans ce navigateur, sur cet ordinateur. Lisez [où vivent vos projets](/docs/welcome/where-your-projects-live) avant d’y passer des heures.

## le bandeau ouvrir dans l’application {#the-open-on-desktop-band}

Sur un ordinateur, la version web affiche un bandeau en haut : **ethereal fonctionne mieux dans l’application de bureau**, avec **ouvrir dans l’application**. Ce bouton ouvre l’application de bureau si elle est installée. S’il ne se passe rien, le bandeau propose **télécharger Ethereal** à la place. À côté, **discord** ouvre le Discord de la communauté Ethereal dans un nouvel onglet (l’écran d’accueil a le même lien, sous forme d’icône Discord).

Ouvrir l’application de bureau ne déplace pas vos projets web : les deux gardent des projets séparés (voir [déplacer des projets](/docs/basics/moving-projects)). Le **×** masque le bandeau jusqu’au prochain rechargement de la page.

## téléphones et tablettes {#phones-and-tablets}

Sur un téléphone, ou une tablette sans souris ni pavé tactile, la version web affiche « ethereal n’est pas disponible sur mobile — essayez-le sur ordinateur », avec **copier le lien** (pour ouvrir le même lien sur un ordinateur) et **essayer la bêta mobile**. La bêta est une version tactile de l’application, en développement : voir [Ethereal sur votre téléphone](/docs/mobile/mobile-beta).

Une tablette avec un clavier et un pavé tactile compte comme un ordinateur. Les liens de morceaux partagés (`/s/…`) et cette documentation fonctionnent aussi sur téléphone.

Les gros projets sont lourds pour un navigateur : l’application de bureau a plus de place pour beaucoup de pistes et d’appareils.

## obtenir l’application de bureau {#getting-the-desktop-app}

[La page de téléchargement](/download) choisit le bon fichier pour votre ordinateur et liste toutes les plateformes.

## mises à jour {#updates}

L’application de bureau cherche une nouvelle version quelques secondes après son démarrage, puis toutes les 6 heures. Quand il y en a une, une carte affiche **nouveautés**, **installer et redémarrer**, **plus tard** et **ignorer cette version**. Les mises à jour sont vérifiées par signature avant d’être installées. Vous pouvez aussi vérifier vous-même depuis le menu du projet (**rechercher des mises à jour…**), et désactiver la vérification automatique dans réglages › général › **mises à jour**.

Sous Linux, si vous avez installé Ethereal depuis un paquet du système, il ne peut pas se mettre à jour lui-même : la carte le dit, et **télécharger** ouvre la page de la version pour récupérer le nouveau paquet.

La version web est toujours la plus récente : rechargez la page.

## et ensuite {#where-next}

- [Où vivent vos projets](/docs/welcome/where-your-projects-live) : la chose à savoir avant de commencer.
- [Votre première boucle](/docs/welcome/first-beat) : faites de la musique en cinq minutes.
- [Une visite de l’écran](/docs/basics/the-screen) : à quoi sert chaque partie de la fenêtre.
