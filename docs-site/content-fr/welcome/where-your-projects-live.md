---
title: où vivent vos projets
order: 20
summary: Où l’application de bureau et la version web gardent vos projets, et comment ne pas les perdre.
platform: all
audience: beginner
covers:
  - projects-storage/where-your-projects-are-stored #on-the-desktop
  - projects-storage/web-projects-can-be-lost-with-browser-data #in-the-browser
  - remote-engine/web-projects-live-in-the-browser #in-the-browser
  - first-run/only-one-browser-tab-can-use-ethereal #one-tab-at-a-time
---

Ethereal enregistre votre travail au fur et à mesure : il n’y a pas de fichier « enregistrer sous » à choisir. L’endroit où il garde vos projets dépend de la façon dont vous l’utilisez, et les deux emplacements sont séparés. Lisez cette page une fois avant de commencer un projet qui compte pour vous.

## sur l’ordinateur [[desktop]] {#on-the-desktop}

L’application de bureau garde chaque projet dans un dossier, sous **Documents › Ethereal › Projects** dans votre dossier personnel. C’est un dossier ordinaire sur votre disque : vos sauvegardes habituelles (Time Machine, l’historique des fichiers, un dossier synchronisé) l’incluent.

Vos enregistrements sont stockés dans le dossier du projet. Les échantillons que vous faites glisser depuis votre bibliothèque ou un dossier de votre disque restent où ils sont, et le projet pointe vers eux (voir [déplacer des projets](/docs/basics/moving-projects#samples-from-your-disk)).

## dans le navigateur [[web]] {#in-the-browser}

La version web garde vos projets dans le stockage privé de ce navigateur pour le site, sur cet ordinateur uniquement. Ce ne sont pas des fichiers visibles sur votre disque, l’application de bureau ne les voit pas, et un autre navigateur (ou un autre ordinateur) a sa propre liste, séparée.

> [!warning]
> Les projets web ne vivent que dans ce navigateur. Effacer les données ou les cookies du site, fermer une fenêtre de navigation privée, ou le navigateur qui libère de l’espace quand votre disque est plein peuvent les supprimer. Exportez de temps en temps les projets qui comptent pour vous (menu du projet › **exporter le projet…**), ou utilisez l’application de bureau.

Pour limiter ce risque, Ethereal demande au navigateur de conserver son stockage au démarrage. Certains navigateurs acceptent tout de suite, d’autres décident selon votre utilisation du site (un favori ou une application installée aide), et Firefox vous pose la question. Si le navigateur refuse, Ethereal vous le dit une fois. Réglages › général › **stockage** indique où vous en êtes :

- **persistant** : le navigateur garde vos projets.
- **peut être effacé par le navigateur quand l’espace vient à manquer** : avec **redemander** pour demander une nouvelle fois.
- **ce navigateur ne peut pas garder le stockage de façon permanente**.

Il indique aussi l’espace occupé par vos projets.

## déplacer un projet {#moving-a-project}

Pour faire passer un projet du navigateur à l’application de bureau, ou à un autre ordinateur, exportez-le en fichier `.ether` et importez-le là-bas. Voir [déplacer des projets entre ordinateurs](/docs/basics/moving-projects).

## un onglet à la fois [[web]] {#one-tab-at-a-time}

Un seul onglet du navigateur peut travailler sur vos projets à la fois. Si vous ouvrez Ethereal dans un deuxième onglet, il affiche **ethereal est ouvert dans un autre onglet** avec **utiliser ici**. **Utiliser ici** fait passer Ethereal dans le nouvel onglet : le premier onglet enregistre votre travail, s’arrête et affiche **ethereal est passé dans un autre onglet** (avec son propre **utiliser ici** pour le ramener). Si le premier onglet ne répond pas en quelques secondes, parce qu’il est figé, le nouvel onglet prend le relais quand même.

L’application de bureau n’a pas cette limite, et cette documentation peut être ouverte dans autant d’onglets que vous voulez.

## et ensuite {#where-next}

- [Votre première boucle](/docs/welcome/first-beat) : faites de la musique en cinq minutes.
- [Projets et enregistrement](/docs/basics/projects) : l’écran d’accueil, le menu du projet, l’enregistrement automatique.
- [Versions et sauvegardes](/docs/basics/versions-and-backups) : revenez à un état antérieur d’un projet.
