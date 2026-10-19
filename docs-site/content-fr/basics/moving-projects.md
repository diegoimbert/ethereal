---
title: déplacer des projets entre ordinateurs
order: 60
summary: Exportez un projet dans un seul fichier .ether, ouvrez-le sur un autre ordinateur ou entre la version web et l’application de bureau, et retrouvez les échantillons manquants.
platform: all
audience: beginner
covers:
  - projects-storage/moving-a-project-between-web-and-desktop-or-to-another-compu #moving-a-project
  - projects-storage/export-project-pack-media-files-or-not #exporting-a-project
  - projects-storage/missing-samples-and-relinking #missing-samples
  - projects-storage/samples-used-from-your-disk-aren-t-inside-the-project #samples-from-your-disk
---

Vos projets sont stockés sur l’ordinateur, ou dans le navigateur, où vous les avez créés (voir [où vivent vos projets](/docs/welcome/where-your-projects-live)). Pour travailler sur un morceau ailleurs, entre la version web et l’application de bureau, ou simplement pour en garder une sauvegarde, vous l’exportez dans un fichier **.ether** et vous importez ce fichier de l’autre côté.

## déplacer un projet {#moving-a-project}

1. Sur le premier ordinateur, ouvrez le projet et choisissez **exporter le projet…** dans le menu du projet (☰). Voir [exporter un projet](#exporting-a-project) plus bas.
2. Transférez le fichier .ether sur l’autre ordinateur (une clé USB, un stockage en ligne, un e-mail…).
3. Sur l’autre ordinateur, choisissez **importer un projet…** dans le menu du projet, ou déposez simplement le fichier .ether sur la fenêtre d’Ethereal.

Le projet importé est ajouté comme nouveau projet et s’ouvre. Si vous avez déjà un projet du même nom, le nouveau reçoit un numéro, comme « Song 2 ». Rien de ce que vous avez déjà n’est remplacé.

Cela marche dans tous les sens : du bureau au bureau, du web au bureau, du bureau au web.

## exporter un projet {#exporting-a-project}

**Exporter le projet…** (dans le menu du projet, ou dans le menu d’un projet récent sur l’écran d’accueil) ouvre une petite fenêtre avec un seul choix, **inclure les fichiers média** :

- **Activé** (par défaut) : un seul fichier .ether autonome, avec le projet et tous les fichiers audio qu’il utilise, y compris les échantillons de votre bibliothèque ou d’ailleurs sur votre disque. Utilisez-le pour déplacer un projet ou en garder une sauvegarde.
- **Désactivé** : seulement le projet lui-même, beaucoup plus léger. Les fichiers audio ne sont pas inclus, donc sur un autre ordinateur les échantillons apparaissent comme manquants.

Appuyez sur **exporter**. L’application de bureau vous demande où enregistrer le fichier ; le navigateur le télécharge.

> [!tip]
> Exporter avec **inclure les fichiers média** activé est aussi la meilleure façon de garder une sauvegarde d’un projet web en dehors du navigateur.

## échantillons de votre disque {#samples-from-your-disk}

Quand vous faites glisser un échantillon dans un projet depuis votre bibliothèque ou depuis un dossier de votre disque, Ethereal ne le copie pas dans le projet : le projet pointe vers le fichier là où il se trouve. Si vous déplacez, renommez ou supprimez ce fichier plus tard, l’échantillon devient manquant.

Pour rendre un projet indépendant de ces fichiers, utilisez **tout rassembler et enregistrer** dans la palette de commandes (ou dans la fenêtre des échantillons manquants). Cela copie dans le dossier du projet tous les échantillons qu’il utilise depuis l’extérieur, puis enregistre. C’est une seule étape d’annulation.

## échantillons manquants {#missing-samples}

Quand un projet ne trouve pas certains de ses fichiers audio, un avis indique **1 échantillon manquant** (ou **N échantillons manquants**) avec un bouton **relier…**. Les clips qui utilisent un fichier manquant jouent du silence jusqu’à ce que vous le retrouviez.

**Relier…** ouvre la fenêtre **échantillons manquants**. Elle liste chaque fichier manquant avec un badge **manquant** ou **trouvé**, et vous propose plusieurs façons de les retrouver :

- **localiser…** sur une ligne : choisissez le fichier vous-même ;
- **chercher dans la bibliothèque** : parcourt votre bibliothèque à la recherche des fichiers manquants ;
- **chercher dans un dossier…** [[desktop]] : parcourt un dossier de votre choix.

Une recherche cherche des fichiers du même nom. Un fichier au contenu strictement identique est relié aussitôt. Un fichier du même nom mais au contenu différent est listé sous son échantillon avec un bouton **utiliser**, pour que vous décidiez.

La fenêtre propose aussi **tout rassembler et enregistrer** (voir [plus haut](#samples-from-your-disk)).

Vous pouvez aussi ouvrir la fenêtre avec **relier les échantillons manquants…** dans la palette de commandes, ou faire un clic droit sur un clip dont l’échantillon manque et choisir **relier l’échantillon…**.

Pour les plug-ins et appareils manquants aussi, voir [plug-ins et échantillons manquants](/docs/sounds-and-plugins/missing-plugins-and-samples).

## et ensuite {#where-next}

- [Versions et sauvegardes](/docs/basics/versions-and-backups)
- [Où vivent vos projets](/docs/welcome/where-your-projects-live)
- [Plug-ins et échantillons manquants](/docs/sounds-and-plugins/missing-plugins-and-samples)
