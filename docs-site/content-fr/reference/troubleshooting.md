---
title: dépannage
order: 30
summary: Que vérifier quand il n’y a pas de son, que l’audio craque, que le moteur se bloque, qu’un plug-in se comporte mal ou que vos projets web sont menacés.
platform: all
audience: everyone
covers:
  - app-shell/engine-not-responding-warning #the-engine-is-not-responding
  - app-shell/engine-log-desktop #the-engine-log
---

Trouvez votre problème dans les titres ci-dessous. Chaque partie explique ce qui se passe en général et quoi essayer. Si rien ici ne vous aide, [signalez un problème](https://github.com/diegoimbert/ethereal/issues) en précisant votre configuration.

## pas de son {#no-sound}

Vérifiez ces points dans l’ordre :

1. **Est-ce que ça joue ?** Appuyez sur [[action:transport.play]] et regardez la tête de lecture avancer. Si elle ne bouge pas du tout, voir [le moteur ne répond pas](#the-engine-is-not-responding).
2. **Les vumètres bougent-ils ?** Les en-têtes de piste affichent un vumètre. Aucun niveau sur une piste signifie que rien ne lui arrive : vérifiez que le clip n’est pas muet, que la piste n’est pas muette, et qu’aucune autre piste n’est en solo.
3. **La sortie est-elle la bonne ?** [[desktop]] Ouvrez les **réglages** (la roue dentée à droite de la barre d’état, en bas), puis **audio**, et vérifiez le **périphérique de sortie**. Utilisez le bouton **actualiser les périphériques** après avoir branché une interface ou un casque.
4. **Dans le navigateur** [[web]], c’est le système qui choisit le périphérique de sortie : changez-le dans votre système d’exploitation ou votre navigateur, pas dans Ethereal. Les navigateurs n’autorisent le son qu’après une interaction avec la page : cliquez ou appuyez une fois sur une touche si la première lecture est silencieuse.

### le périphérique de sortie a disparu [[desktop]] {#the-output-device-disappeared}

Si vous débranchez votre interface audio ou éteignez votre casque Bluetooth pendant qu’Ethereal tourne, un avertissement nomme le périphérique. Ethereal continue sur la sortie configurée s’il le peut, sinon sur la sortie par défaut du système, et sinon sans son (l’application continue de fonctionner). Il recherche votre périphérique et y revient dès qu’il est rebranché. Vos réglages enregistrés ne changent pas.

### « périphérique fermé » [[desktop]] {#device-closed}

Si l’onglet audio indique **périphérique fermé : aucun moteur local ne joue**, cette fenêtre joue à travers un [serveur de moteur](/docs/advanced/engine-server). Votre périphérique audio se rouvre, avec les mêmes réglages, quand vous revenez à votre propre moteur.

## craquements et coupures {#crackles-and-dropouts}

Un craquement ou un court trou est une **coupure** : l’ordinateur n’a pas fini à temps le morceau d’audio suivant. La ligne d’état dans **réglages › audio** les compte (**… coupures**) à côté de la fréquence d’échantillonnage, de la taille du buffer et de la latence.

- **Augmentez la taille du buffer** dans **réglages › audio**. Un buffer plus grand donne à l’ordinateur plus de temps pour chaque morceau, au prix d’un retard plus long entre le moment où vous jouez une touche et celui où vous l’entendez. Baissez-la pour enregistrer, augmentez-la pour mixer.
- **Gelez les pistes lourdes** : geler fait le rendu d’une piste pour que ses appareils n’utilisent plus le processeur. Voir [geler, aplatir et faire un rendu](/docs/advanced/freeze-and-bounce).
- **Fermez les autres applications audio** et tout ce qui est lourd en arrière-plan.
- Sur un portable, branchez l’alimentation : l’économie de batterie ralentit le processeur.

La taille de buffer que vous choisissez est une demande : certains périphériques tournent avec une autre taille. L’onglet audio montre celle que le périphérique utilise réellement. Voir [configuration audio et MIDI](/docs/making-music/audio-and-midi-setup).

## le moteur ne répond pas {#the-engine-is-not-responding}

Quand le moteur (la partie qui produit le son) se bloque, un avertissement indique **le moteur ne répond pas**, avec la raison quand il la connaît, par exemple un plug-in occupé à ouvrir son éditeur. L’audio continue de jouer, et ce que vous faites entre-temps s’exécute dès qu’il se rétablit. Quand c’est terminé, un court message indique combien de temps cela a duré.

Un blocage d’une ou deux secondes pendant l’ouverture d’un gros plug-in est normal. Si cela arrive souvent, ou ne s’arrête jamais :

- Consultez le [journal du moteur](#the-engine-log) [[desktop]] : il dit pourquoi le moteur s’est bloqué.
- Si un plug-in en est la cause, essayez de l’exécuter isolé (macOS et Linux), ou ouvrez le projet en [mode sans échec](/docs/advanced/safe-mode) (voir [les plug-ins plantent ou ne se chargent pas](#plugins-crash-or-fail-to-load)).
- [Signalez un problème](https://github.com/diegoimbert/ethereal/issues) avec ce que dit le journal du moteur.

### le moteur audio n’a pas démarré {#the-audio-engine-didnt-start}

Si Ethereal ne parvient pas du tout à démarrer son moteur, il affiche **le moteur audio n’a pas démarré** avec la raison. Vos projets sont en sécurité : rien n’a été ouvert. Utilisez **réessayer** ou **recharger**, et [signalez un problème](https://github.com/diegoimbert/ethereal/issues) si cela se reproduit. [[web]] Si la page indique **une nouvelle version d’ethereal est disponible**, rechargez la page pour l’obtenir.

## le journal du moteur [[desktop]] {#the-engine-log}

**Réglages › avancé › diagnostic › journal du moteur › ouvrir le journal du moteur** ouvre une fenêtre de terminal qui suit le journal du moteur pendant qu’il tourne. Il montre les avertissements, les problèmes de plug-ins et pourquoi le moteur a cessé de répondre, le cas échéant. Quand vous signalez un problème, copiez-en la fin dans votre signalement.

## les plug-ins plantent ou ne se chargent pas [[desktop]] {#plugins-crash-or-fail-to-load}

Les plug-ins sont réservés au bureau. La version web n’a que les appareils intégrés.

- **Un plug-in n’apparaît pas** : ouvrez **réglages › plug-ins**, vérifiez que son dossier est analysé et que les bons formats sont activés, puis **réanalyser**. **Plug-ins en échec (…)…** liste les plug-ins qui n’ont pas pu se charger, avec la raison ; faites un clic droit sur l’un d’eux pour réessayer avec un délai plus long. Voir [plug-ins](/docs/sounds-and-plugins/plugins).
- **Un plug-in plante** : sur macOS et Linux, faites un clic droit sur son en-tête et choisissez **exécuter isolé (protégé des plantages, +1 bloc de latence)**. Il tourne alors dans son propre processus : s’il plante, l’appareil indique qu’il a planté et est contourné, avec un bouton de rechargement, et le reste d’Ethereal continue de tourner.
- **Un projet plante dès son ouverture** : ouvrez-le sans plug-ins. Faites un clic droit sur le projet dans la liste des projets récents (ou utilisez la fenêtre de récupération après plantage) et choisissez **ouvrir sans plug-ins**. Chargez ensuite les plug-ins un par un pour trouver le coupable. Voir [mode sans échec](/docs/advanced/safe-mode).
- **Un plug-in manque** sur cet ordinateur : l’appareil garde son état enregistré et est contourné, donc la piste joue sans effet jusqu’à ce que vous installiez le plug-in et relanciez l’analyse.

## l’enregistrement est silencieux [[desktop]] {#recording-is-silent}

- Vérifiez qu’un **périphérique d’entrée** est choisi dans **réglages › audio**. Avec **aucun (pas d’enregistrement)**, les prises sont silencieuses.
- Armez la piste et activez son monitoring : le **contrôle de l’entrée** en bas de l’onglet audio affiche alors son niveau.
- Sur macOS, si la prise est complètement silencieuse, le système a probablement refusé l’accès au micro. Ethereal vous avertit quand une prise n’est que du silence. Autorisez Ethereal dans **Réglages Système › Confidentialité et sécurité › Microphone**, puis enregistrez de nouveau.

L’enregistrement demande l’application de bureau. Voir [enregistrement](/docs/making-music/recording).

## claviers et contrôleurs midi {#midi-keyboards-and-controllers}

- [[desktop]] Il n’y a pas de liste de périphériques MIDI à configurer : chaque entrée MIDI connectée s’ouvre d’elle-même. Chaque piste choisit les ports et canaux qu’elle écoute dans son panneau **IN ▾**. Si une piste ignore votre clavier, vérifiez ce filtre, et que la piste est armée ou en monitoring.
- [[web]] La version web n’a pas encore de MIDI matériel. Utilisez le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard) ([[action:midi.computerKeyboard]]) ou l’application de bureau.
- Tant que le clavier MIDI de l’ordinateur est activé, les raccourcis à une lettre jouent des notes à la place. Appuyez sur [[action:midi.computerKeyboard]] pour le désactiver.
- Pour contrôler des boutons depuis votre contrôleur, voir [apprentissage MIDI](/docs/advanced/midi-learn).

## vos projets web et le stockage [[web]] {#your-web-projects-and-storage}

La version web garde vos projets dans le stockage de ce navigateur, sur cet ordinateur uniquement. Effacer les données du site, utiliser une fenêtre de navigation privée, ou le navigateur qui libère de l’espace peut les supprimer.

- Quand Ethereal démarre dans le navigateur, il demande au navigateur de conserver son stockage. Si le navigateur refuse, un avis vous le signale une fois. **Réglages › général › bibliothèque › stockage** affiche l’état actuel (**persistant**, ou **peut être effacé par le navigateur quand l’espace vient à manquer**), l’espace qu’occupent vos projets, et **redemander**.
- Chaque navigateur décide à sa façon : Chrome et Edge acceptent plus volontiers pour un site que vous avez installé ou mis en favori, Firefox vous pose la question, et Safari a ses propres règles.
- Quel que soit l’état, gardez des sauvegardes des projets qui comptent : exportez-les depuis le menu du projet, ou utilisez l’application de bureau. Voir [où vivent vos projets](/docs/welcome/where-your-projects-live) et [déplacer des projets entre ordinateurs](/docs/basics/moving-projects).

> [!warning]
> Les projets web ne vivent que dans ce navigateur. Exportez de temps en temps ceux auxquels vous tenez.

## « ethereal est ouvert dans un autre onglet » [[web]] {#ethereal-is-open-in-another-tab}

Un seul onglet du navigateur à la fois peut travailler sur vos projets. Un deuxième onglet affiche **ethereal est ouvert dans un autre onglet** avec **utiliser ici**. **Utiliser ici** demande au premier onglet d’enregistrer votre travail et de se retirer, et cet onglet affiche alors **ethereal est passé dans un autre onglet**. Si le premier onglet ne répond pas en quelques secondes, le nouvel onglet prend la main malgré tout.

## et ensuite {#where-next}

- [Signaler un problème](https://github.com/diegoimbert/ethereal/issues) quand rien ici n’a réglé le problème.
- [Configuration audio et MIDI](/docs/making-music/audio-and-midi-setup) pour le détail des réglages audio.
- [FAQ](/docs/reference/faq).
