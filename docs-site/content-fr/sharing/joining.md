---
title: rejoindre une session
order: 30
summary: Ouvrez un lien d’invitation dans l’application ou dans votre navigateur, choisissez ce qui joue le projet, et réglez une connexion qui échoue.
platform: all
audience: beginner
covers:
  - sharing/joining-open-in-the-app-or-continue-in-browser #opening-an-invite-link
  - sharing/the-join-screen-and-play-with #the-join-screen
  - sharing/why-a-join-fails #when-a-join-fails
  - sharing/the-host-must-be-online-to-join #the-host-must-be-online
---

Quelqu’un vous a envoyé un lien vers son projet. Cette page vous guide pour le rejoindre, et vous dit quoi faire si ça ne marche pas. Vous n’avez pas besoin de compte. Pour ce qui se passe une fois dans la session, voir [sessions en direct](/docs/sharing/live-sessions).

## ouvrir un lien d’invitation {#opening-an-invite-link}

Cliquez sur le lien. Dans un navigateur, vous arrivez sur une page avec deux boutons :

- **ouvrir dans l’application** : ouvre l’invitation dans l’application de bureau Ethereal, si elle est installée. S’il ne se passe rien au bout d’un moment, la page propose de télécharger l’application.
- **continuer dans le navigateur** : rejoint ici même, dans l’application web. Activez **toujours continuer dans le navigateur** pour sauter cette page la prochaine fois.

Vous pouvez aussi coller un lien dans Ethereal avec **rejoindre une session…** : sur l’écran d’accueil, dans le menu du projet, dans la fenêtre de partage et dans la palette de commandes.

> [!note]
> Sur un téléphone, l’application web affiche son écran « pas disponible sur mobile », sauf si vous utilisez la bêta mobile : voir [ethereal sur votre téléphone](/docs/mobile/mobile-beta).

## l’écran pour rejoindre {#the-join-screen}

Avant tout téléchargement, l’écran pour rejoindre indique qui vous invite et à quel projet, si **vous pouvez modifier** ou **vous pouvez écouter**, et qui d’autre est là. Si vous n’avez pas encore choisi de nom, tapez-en un dans **rejoindre en tant que**.

**jouer avec** choisit quel ordinateur joue le projet pour vous :

| choix | ce qu’il fait |
|---|---|
| **mon moteur** (par défaut) | votre ordinateur joue le projet, comme n’importe lequel de vos projets. |
| moteur de *nom* (distant) | vous entendez l’ordinateur de cette personne ; le vôtre ne charge pas le projet. On lui demande d’abord son accord. Voir [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together). |
| lancer un moteur pour moi sur l’ordinateur de *nom* | la personne démarre un second moteur rien que pour vous, avec ses plug-ins. On lui demande d’abord son accord. |
| **aucun moteur (lecture seule)** | le projet s’ouvre mais rien ne joue : pour un projet trop lourd pour votre ordinateur. |

Cliquez sur **rejoindre**. Ethereal télécharge le projet et ses fichiers audio, avec une barre de progression, puis l’ouvre. **pas maintenant** ferme l’écran sans rejoindre.

Si vous avez déjà une copie plus ancienne du projet, l’écran le signale : elle est mise à jour, et les modifications que vous avez faites hors ligne sont gardées dans un projet séparé « (copie locale) ».

## l’hôte doit être en ligne {#the-host-must-be-online}

Les projets vivent sur l’ordinateur de l’hôte, pas dans un cloud. Personne ne peut rejoindre tant que l’Ethereal de l’hôte est fermé : l’écran pour rejoindre indique « l’Ethereal de l’hôte est fermé » et attend, et le projet s’ouvre dès son retour. Si vous avez déjà rejoint auparavant, **ouvrir ma copie hors ligne** vous permet de travailler en attendant.

Un hôte sur l’application web se déconnecte quand il ferme l’onglet ou que son portable se met en veille.

## quand la connexion échoue {#when-a-join-fails}

L’écran pour rejoindre indique **impossible de rejoindre** et la raison :

| message | que faire |
|---|---|
| ce lien d’invitation a été réinitialisé ou n’est plus valide | demandez un nouveau lien à l’hôte. |
| le lien d’invitation est endommagé | le lien a été coupé ou modifié en le copiant : copiez-le à nouveau, en entier. |
| la session est pleine | 16 personnes sont déjà en ligne. Réessayez plus tard. |
| ce projet a atteint sa limite de 64 membres | le projet se souvient de 64 personnes qui l’ont rejoint. Demandez à l’hôte de retirer quelqu’un de la liste des participants. |
| trop de personnes rejoignent ce projet en ce moment | le service de partage limite les connexions pendant un instant. Réessayez dans une minute. |
| mettez à jour Ethereal pour rejoindre ce projet | vous et l’hôte utilisez des versions différentes. Mettez à jour l’application de bureau (ou demandez à l’hôte de le faire). |
| impossible de joindre l’ordinateur de l’hôte (réseau) | un réseau strict (bureau, école, certains opérateurs mobiles) bloque les connexions directes. Réessayez, ou demandez à l’hôte de configurer un serveur relais (TURN) dans **réglages › avancé**. |
| impossible de joindre le service de partage | vérifiez votre connexion internet et réessayez. |
| l’identité de l’hôte n’a pas pu être vérifiée | n’allez pas plus loin ; demandez à l’hôte de renvoyer le lien par un autre moyen. |
| ce projet est trop volumineux pour être rejoint | l’audio du projet est trop lourd à transférer. Demandez à l’hôte de supprimer l’audio inutilisé. |

## et ensuite {#where-next}

- [sessions en direct](/docs/sharing/live-sessions) : ce qui se synchronise, la discussion, les notes et quitter une session.
- [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together)
