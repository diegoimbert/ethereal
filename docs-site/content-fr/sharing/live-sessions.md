---
title: sessions en direct
order: 20
summary: Démarrez une session en direct, gérez qui y participe, et comprenez ce que tout le monde partage et ce qui reste à vous.
platform: all
audience: everyone
covers:
  - sharing/allow-editing-toggle-live #allow-editing
  - sharing/reset-link-vs-remove-vs-stop-sharing #reset-remove-or-stop
  - sharing/quitting-is-not-stopping-sharing #closing-the-app-doesnt-stop-sharing
  - sharing/people-list-roles-and-limits #the-people-list
  - sharing/session-pill-and-view-only-badge #the-session-pill
  - sharing/shared-projects-in-your-project-list #shared-projects-in-your-project-list
  - sharing/privacy-of-live-sharing #privacy
  - collaboration/everyone-plays-on-their-own-computer #everyone-plays-on-their-own-computer
  - collaboration/mute-and-solo-are-per-person #mute-and-solo-are-yours
  - collaboration/what-syncs-and-what-stays-local #what-syncs-and-what-stays-yours
  - collaboration/audio-files-are-sent-to-everyone #audio-files-are-sent-to-everyone
  - collaboration/plugins-don-t-travel #plugins-dont-travel
  - collaboration/undo-only-undoes-your-own-changes #undo-only-undoes-your-own-changes
  - collaboration/live-pointers-and-selections #seeing-each-other
  - collaboration/follow-someone-s-view #following-someones-view
  - collaboration/hide-users-and-notes #hide-users-and-notes
  - collaboration/session-chat #chat
  - collaboration/pinned-notes #pinned-notes
  - collaboration/offline-copies-and-host-going-offline #when-the-host-goes-offline
  - collaboration/local-copy-projects #local-copy-projects
  - collaboration/leaving-a-session #leaving-a-session
  - collaboration/when-sharing-ends #when-sharing-ends
  - collaboration/everyone-must-run-the-same-version #everyone-needs-the-same-version
---

Une session en direct permet à d’autres personnes de travailler sur votre projet avec vous, en temps réel, chacune sur son propre ordinateur. Cette page s’adresse à la personne qui partage (l’hôte) et à tous ceux qui rejoignent. Si vous n’avez pas encore choisi de type de partage, commencez par [partager votre musique](/docs/sharing/overview).

## démarrer une session {#start-a-session}

1. Ouvrez le projet et cliquez sur **partager** dans la barre du haut.
2. Dans l’onglet **collaboration en direct**, cliquez sur **démarrer une session en direct**.
3. Choisissez l’onglet **peut modifier** ou **peut écouter**, cliquez sur **copier** et envoyez le lien.

La première fois, la fenêtre vous demande votre nom et votre couleur (**vous apparaissez comme**). Vos amis ouvrent le lien et rejoignent : voir [rejoindre une session](/docs/sharing/joining).

> [!note]
> Personne ne peut rejoindre tant que votre Ethereal est fermé. Sur le web, fermer l’onglet ou laisser le portable se mettre en veille met aussi la session hors ligne.

## autoriser la modification {#allow-editing}

L’interrupteur **autoriser la modification** de la fenêtre masque le lien de modification, pour que seul le lien d’écoute soit proposé. Il ne retire personne : les personnes déjà entrées avec un lien de modification peuvent toujours modifier. Pour retirer des droits de modification, réinitialisez le lien de modification et changez le rôle de ces personnes ou retirez-les (voir plus bas). L’interrupteur est enregistré avec les réglages de partage du projet, il reste donc tel que vous l’avez laissé quand vous rechargez ou rouvrez le projet. Arrêter le partage le réinitialise.

## la liste des participants {#the-people-list}

La fenêtre liste toutes les personnes qui ont rejoint le projet, en ligne ou non, avec un point pour celles qui sont en ligne maintenant et « vu pour la dernière fois… » pour les autres. L’hôte peut changer le rôle d’une personne (**peut modifier** ou **peut écouter**) et, depuis le menu **…** de la ligne, choisir **retirer du projet**.

Une session accueille au plus 16 personnes en ligne à la fois, et un projet partagé retient au plus 64 membres. Au-delà, la demande est refusée, et la personne qui essaie de rejoindre apprend quelle limite est atteinte (« la session est pleine » ou « ce projet a atteint sa limite de 64 membres »).

## masquer les appareils aux auditeurs {#hiding-devices-from-listeners}

**masquer les appareils aux auditeurs**, dans la fenêtre de partage, garde vos appareils privés pour les personnes qui ont rejoint avec un lien d’écoute : elles reçoivent le projet sans appareils, sans plug-ins ni leurs réglages (ni l’automation et les mappings MIDI qui vont avec). Les personnes qui ont le lien de modification voient toujours tout. L’activer ou le désactiver reconnecte les auditeurs pour qu’ils reçoivent tout de suite la bonne version.

## réinitialiser, retirer ou arrêter {#reset-remove-or-stop}

Il y a trois niveaux, du plus petit au plus grand :

| action | où | ce qui se passe |
|---|---|---|
| **réinitialiser le lien de modification** / **réinitialiser le lien d’écoute** | le menu **…** en bas de la fenêtre | l’ancien lien ne fonctionne plus. Les personnes déjà entrées gardent leur accès, car chaque membre a une clé personnelle. |
| **retirer du projet** | le menu **…** de la personne dans la liste des participants | seule cette personne perd l’accès. Elle ne peut revenir qu’avec un lien à jour. |
| **arrêter le partage** | le bouton en bas de la fenêtre | tous les liens et toutes les clés des membres cessent de fonctionner. Chaque personne qui a rejoint garde une copie hors ligne. |

Pour exclure quelqu’un qui a un lien que vous avez fait circuler, faites les deux : réinitialisez le lien, puis retirez la personne.

## fermer l’application n’arrête pas le partage {#closing-the-app-doesnt-stop-sharing}

Fermer Ethereal ou ouvrir un autre projet ne fait que mettre la session en pause. Les liens restent valides, les personnes de la session voient « *hôte* est passé hors ligne », et quand vous rouvrez le projet la session reprend d’elle-même. Seul **arrêter le partage** y met fin.

Si vous préférez repartager vous-même à chaque fois, désactivez **reprendre le partage quand j’ouvre un projet partagé** dans **réglages › partage**.

## la pastille de session {#the-session-pill}

Tant qu’un projet est partagé (ou que vous en avez rejoint un), le bouton **partager** devient une pastille avec les avatars des personnes en ligne et un statut :

- **en direct** : connecté, les modifications se synchronisent.
- **connexion…** ou **reconnexion…** : en cours, ou de retour après une coupure de connexion.
- **impossible à rejoindre** (hôte) : le service de partage est injoignable. Les personnes déjà présentes restent connectées, mais personne de nouveau ne peut rejoindre.
- le nom de l’hôte suivi de **hors ligne** (invité) : vous travaillez sur une copie hors ligne (voir plus bas).

Cliquez sur la pastille pour ouvrir la fenêtre. Les invités qui ont rejoint avec un lien d’écoute voient aussi un badge **lecture seule**.

## chacun joue sur son propre ordinateur {#everyone-plays-on-their-own-computer}

Chaque personne a sa propre copie du projet et son propre moteur audio. La lecture est personnelle : lecture et arrêt, la tête de lecture, la boucle et le métronome ne concernent que vous. Lancer la lecture ne la lance pas chez les autres, et leur lecture ne déplace pas la vôtre. Les têtes de lecture des autres s’affichent comme de fines lignes colorées avec leurs initiales.

Pour entendre exactement ce qu’une autre personne entend, écoutez sur son ordinateur : voir [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together).

## muet et solo sont à vous {#mute-and-solo-are-yours}

Muet et solo sur les pistes et les pads de batterie sont personnels, comme votre propre casque : ils ne sont jamais envoyés aux autres. Quand vous rejoignez, vous partez une fois des muet et solo de l’hôte, puis chacun gère ses muet et solo de son côté. L’exception : pendant que vous écoutez sur l’ordinateur de quelqu’un, vous entendez (et contrôlez) ses muet et solo.

## ce qui se synchronise et ce qui reste à vous {#what-syncs-and-what-stays-yours}

| synchronisé avec tout le monde | reste sur votre ordinateur |
|---|---|
| pistes, clips, notes, appareils et leurs paramètres, automation | lecture, arrêt, enregistrement et la tête de lecture |
| volume, panoramique, routage, envois, noms, couleurs, ordre des pistes | la boucle et le métronome |
| tempo, signatures rythmiques, marqueurs, marqueurs de warp | muet, solo et armement |
| mappings MIDI, pads de batterie | votre sélection et votre historique d’annulation |
| la discussion et les notes épinglées | vos réglages audio et vos préférences |
| le nom du projet et le swing | |

## les fichiers audio sont envoyés à tout le monde {#audio-files-are-sent-to-everyone}

Quand quelqu’un importe de l’audio ou enregistre une prise, le fichier est envoyé à tout le monde avant que le clip n’apparaisse. Les personnes qui rejoignent plus tard téléchargent d’abord tout l’audio : c’est la barre de progression de l’écran pour rejoindre. Une session contient environ 1 Go d’audio. Un très gros projet peut échouer avec « ce projet est trop volumineux pour être rejoint ».

## les plug-ins ne voyagent pas {#plugins-dont-travel}

Les plug-ins ne sont jamais partagés, seuls leurs réglages le sont. Si quelqu’un n’a pas un plug-in que vous utilisez, il est contourné de son côté (il entend cette piste sans effet ou muette), et cela ne vous affecte jamais. Les changements faits dans la fenêtre propre d’un plug-in (un preset chargé dans le plug-in, par exemple) arrivent chez les autres quand le projet de cette personne est enregistré, à la main ou par l’enregistrement automatique.

Pour faire quand même entendre vos plug-ins à quelqu’un, cette personne peut écouter sur votre ordinateur : voir [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together).

## annuler n’annule que vos propres modifications {#undo-only-undoes-your-own-changes}

Dans une session, [[action:edit.undo]] n’annule que vos propres modifications, jamais celles des autres. Si un ami a changé la même chose après vous, votre annulation laisse son changement en place. Annuler à l’intérieur de quelque chose qu’un ami a supprimé entre-temps est aussi ignoré. Annuler peut donc parfois sembler ne rien faire : c’est le changement plus récent de quelqu’un d’autre qui l’emporte.

## se voir les uns les autres {#seeing-each-other}

Dans l’arrangement, vous voyez les pointeurs de souris des autres avec leur nom dans leur couleur, leurs clips et pistes sélectionnés entourés de leur couleur, et un anneau autour des clips que quelqu’un est en train de modifier. Rien de tout cela n’est un bug, et rien ne modifie votre projet.

## suivre la vue de quelqu’un {#following-someones-view}

Cliquez sur l’avatar d’une personne dans la barre du haut, ou choisissez **suivre** et son nom dans son menu **…** de la fenêtre de partage. Votre arrangement défile et zoome alors avec le sien, et quand elle ouvre un clip dans le piano roll, il s’ouvre aussi chez vous. Suivre ne change jamais votre lecture.

Le suivi s’arrête dès que vous faites défiler ou zoomez vous-même, appuyez sur [[key:Escape]] ou cliquez à nouveau sur l’avatar.

## masquer les participants et les notes {#hide-users-and-notes}

Trop chargé ? Activez **masquer les participants et les notes** : dans la fenêtre sous **écoute et visibilité** (hôte) ou **visibilité** (invités). Cela masque les pointeurs, têtes de lecture, sélections et notes épinglées des autres, sur votre écran uniquement. Les modifications, la discussion et la liste des participants continuent de fonctionner.

## discussion {#chat}

Pendant une session, un onglet **discussion** apparaît dans la barre latérale gauche. [[action:collab.focusChat]] amène au champ de message. [[key:Enter]] envoie et [[key:Shift+Enter]] passe à la ligne ; un message contient jusqu’à 2 000 caractères. Les invités avec un lien d’écoute peuvent aussi discuter.

Les messages sont enregistrés dans le projet (les 2 000 derniers) et réapparaissent à la session suivante. Ils ne peuvent pas être modifiés, supprimés ni annulés.

## notes épinglées {#pinned-notes}

Faites un clic droit sur un endroit vide de l’arrangement (un couloir ou la règle) ou de la grille du piano roll et choisissez **laisser une note** pour y épingler un commentaire. Cliquez sur un point pour le lire. Son menu propose **modifier la note**, **résoudre la note** (ou **rouvrir la note**) et **jeter la note**, et vous pouvez faire glisser un point pour le déplacer.

Toute personne de la session peut modifier, résoudre ou jeter n’importe quelle note, pas seulement les siennes. Un projet contient jusqu’à 500 notes. Les notes fonctionnent aussi hors session, comme pense-bêtes pour vous-même.

## quand l’hôte se déconnecte {#when-the-host-goes-offline}

Si l’hôte ferme Ethereal, les invités continuent de travailler sur une copie hors ligne, avec un bandeau : « *hôte* est hors ligne. Vous travaillez sur une copie hors ligne. Les modifications se synchronisent à son retour ; gardez Ethereal ouvert. » Une connexion coupée se rétablit d’elle-même.

> [!warning]
> Gardez Ethereal ouvert pendant l’absence de l’hôte. Vos modifications en attente se synchronisent à son retour.

## les projets « copie locale » {#local-copy-projects}

Si vous avez modifié un projet partagé hors ligne et que la version de la session a avancé sans vous, votre travail n’y est pas fusionné. À la reconnexion, votre version est enregistrée comme un projet séparé nommé « *morceau* (copie locale) » avec un badge **copie locale**, et la version de la session s’ouvre. La notification indique « modifications hors ligne conservées ». Rien n’est perdu : ouvrez la copie locale et reprenez à la main ce dont vous avez besoin.

## quitter une session {#leaving-a-session}

Invités : **quitter** en bas de la fenêtre. Votre copie reste sur cet ordinateur et apparaît dans vos projets avec un badge **de** qui nomme l’hôte ; la rouvrir vous reconnecte.

Ouvrir, créer ou importer un autre projet pendant une session demande d’abord : « quitter la session *morceau* ? », avec **quitter et ouvrir** (ou **quitter et créer**…). Cela vaut aussi pour l’hôte.

## les projets partagés dans votre liste de projets {#shared-projects-in-your-project-list}

Dans la liste des projets, les projets partagés portent un badge avec les dernières personnes vues :

- **partagé** : vous en êtes l’hôte. Son menu propose **copier le lien d’invitation** et **arrêter le partage…**.
- **de** et le nom de l’hôte : votre copie du projet de quelqu’un d’autre. Son menu propose **se reconnecter** et **faire une copie privée…**.
- **partage terminé** : l’hôte a arrêté de le partager.
- **copie locale** : voir plus haut.

**faire une copie privée…** transforme le projet en projet normal qui ne se synchronise plus avec l’hôte. Dupliquer un projet partagé ou l’enregistrer sous un autre nom n’emporte jamais le partage.

## quand le partage se termine {#when-sharing-ends}

Quand l’hôte arrête le partage ou vous retire, vous recevez « *hôte* a arrêté de partager *morceau*. Votre copie reste sur cet ordinateur. » Le projet affiche **partage terminé** dans votre liste. Utilisez **faire une copie privée…** pour continuer à y travailler de votre côté.

## tout le monde doit avoir la même version {#everyone-needs-the-same-version}

Tous les membres d’une session doivent utiliser la même version d’Ethereal. Si la vôtre est différente, la connexion échoue avec « mettez à jour Ethereal pour rejoindre ce projet ». L’application web est toujours à la dernière version. Sur le bureau, mettez l’application à jour (voir [obtenir l’application de bureau](/docs/welcome/what-is-ethereal)).

## confidentialité {#privacy}

Le projet, la discussion et chaque modification circulent chiffrés, directement entre les ordinateurs (ou via un relais qui ne voit que des données chiffrées). Le service de partage ne fait que présenter les ordinateurs les uns aux autres : il ne voit jamais les noms, le projet ni les messages. Comme dans toute application pair à pair, les personnes d’une session peuvent voir l’adresse IP des autres. Pour masquer la vôtre, activez **masquer mon IP (relais uniquement)** dans **réglages › avancé** ; la connexion passe alors uniquement par un serveur relais.

## et ensuite {#where-next}

- [rejoindre une session](/docs/sharing/joining)
- [écouter sur l’ordinateur d’un ami](/docs/sharing/listening-together)
- [annuler et historique](/docs/basics/undo-and-history)
