---
title: écouter sur l’ordinateur d’un ami
order: 40
summary: Écoutez le moteur d’une autre personne dans une session, lancez un moteur sur son ordinateur, ou rejoignez sans aucun moteur.
platform: all
audience: everyone
covers:
  - collaboration/listen-on-a-friend-s-computer #listening-on-someone
  - collaboration/listening-needs-the-other-person-s-yes #they-say-yes-first
  - collaboration/host-s-listening-settings #your-listening-settings
  - collaboration/listening-latency #latency
  - collaboration/play-with-no-engine-view-only #no-engine-view-only
  - collaboration/run-an-engine-for-me-on-someone-s-computer #running-an-engine-on-their-computer
---

Dans une [session en direct](/docs/sharing/live-sessions), chacun joue normalement le projet sur son propre ordinateur. Cette page couvre les autres options : écouter l’ordinateur d’un ami, emprunter son ordinateur pour faire tourner un moteur pour vous, ou ouvrir un projet sans moteur.

## écouter sur l’ordinateur de quelqu’un {#listening-on-someone}

Ouvrez la fenêtre de partage, trouvez la personne dans la liste des participants et choisissez **écouter sur** suivi de son nom dans son menu **…**. Vous pouvez aussi faire un clic droit sur son avatar dans la barre du haut. Vous entendez alors la sortie de son moteur, diffusée jusqu’à vous, avec une tête de lecture partagée : un statut sous la barre du haut indique « écoute de » et son nom, et **×** arrête.

C’est utile quand elle a un plug-in que vous n’avez pas, ou un ordinateur plus rapide. Pendant l’écoute :

- votre lecture, arrêt, boucle et tête de lecture contrôlent *son* moteur, si elle l’autorise (voir plus bas) ;
- vous entendez ses muet et solo, et vos clics sur muet et solo changent les siens ;
- vous ne pouvez pas enregistrer : Ethereal indique « arrêtez l’écoute pour enregistrer ».

Les invités qui rejoignent avec un lien d’écoute écoutent automatiquement l’hôte. Pour éviter cela, désactivez **écouter automatiquement l’hôte en rejoignant avec un lien d’écoute** dans **réglages › partage**. Dans la fenêtre de partage, les invités en écoute ont **arrêter l’écoute** et **écouter à nouveau**.

## la personne dit oui d’abord {#they-say-yes-first}

La personne que vous voulez entendre reçoit « *nom* veut écouter votre moteur » avec **autoriser** et **refuser**. Sans réponse dans les 30 secondes, la demande est refusée. En attendant sa réponse, vous voyez « connexion à… ». L’autorisation dure jusqu’à la fin de la session, et la personne peut l’arrêter ou la révoquer à tout moment.

## vos réglages d’écoute {#your-listening-settings}

Dans la fenêtre de partage, sous **écoute et visibilité** :

- **laisser les autres écouter mon ordinateur** (activé par défaut).
- **les auditeurs peuvent lire, arrêter et déplacer la tête de lecture** (activé par défaut). Désactivé, les clics d’un auditeur sur lecture, arrêt, muet ou solo sont ignorés.
- les listes des personnes qui vous écoutent et de celles qui y sont autorisées, chacune avec **arrêter** ou **révoquer**.

Pendant que quelqu’un écoute, un badge comme « 1 à l’écoute » s’affiche dans la barre du haut. Quand plusieurs personnes contrôlent le même moteur, la dernière commande l’emporte : il n’y a pas de verrouillage.

## latence {#latency}

Ce que vous entendez pendant l’écoute est un peu en retard sur l’autre ordinateur, environ 60 à 200 millisecondes (réseau, mise en mémoire tampon et votre sortie audio). Ethereal décale votre tête de lecture pour qu’elle corresponde à ce que vous entendez. Cela convient pour mixer et donner son avis, pas pour jouer en rythme par-dessus.

## aucun moteur (lecture seule) {#no-engine-view-only}

Choisissez **aucun moteur (lecture seule)** dans **jouer avec** (sur l’écran pour rejoindre, ou dans la fenêtre de partage pendant une session) pour ouvrir un projet trop lourd pour votre ordinateur. Vous pouvez parcourir le projet, et le modifier si vous en avez le droit, mais la lecture et l’enregistrement sont désactivés et les formes d’onde ne sont pas dessinées. Un badge **aucun moteur — lecture seule** dans la barre du haut propose **utiliser mon moteur** pour revenir en arrière.

Si votre moteur ne parvient pas à charger le projet quand vous rejoignez, Ethereal passe de lui-même dans ce mode et le badge explique pourquoi.

## faire tourner un moteur sur son ordinateur {#running-an-engine-on-their-computer}

**lancer un moteur pour moi sur** suivi d’un nom (dans **jouer avec**) demande à l’application de bureau de cette personne de démarrer un second moteur rien que pour vous, avec son processeur et ses plug-ins. Vous seul l’entendez ; elle non. Il faut que l’autre personne utilise l’application de bureau.

On lui demande d’abord : « *nom* veut lancer un moteur sur votre ordinateur pour son usage (utilise votre processeur et vos plug-ins ; vous ne l’entendrez pas) ». Sous **écoute et visibilité**, elle peut désactiver **laisser les autres lancer un moteur sur mon ordinateur**, choisir combien peuvent tourner à la fois (2 par défaut, jusqu’à 4), voir l’utilisation processeur de chaque moteur et l’**arrêter**.

## et ensuite {#where-next}

- [sessions en direct](/docs/sharing/live-sessions)
- [rejoindre une session](/docs/sharing/joining)
- [le serveur de moteur](/docs/advanced/engine-server) : un moteur sur une autre machine, en dehors des sessions.
