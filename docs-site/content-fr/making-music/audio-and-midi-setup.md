---
title: configuration audio et midi
order: 130
summary: Choisissez votre interface audio, la fréquence d’échantillonnage et la taille du buffer, contrôlez votre entrée et réglez les problèmes audio courants.
platform: all
audience: beginner
covers:
  - audio-midi-setup/settings-audio-driver-output-input-sample-rate-buffer #the-audio-settings
  - audio-midi-setup/in-the-browser-the-system-picks-the-devices #in-the-browser
  - audio-midi-setup/buffer-size-what-you-ask-for-vs-what-the-device-runs #what-you-ask-for-and-what-the-device-runs
  - audio-midi-setup/smaller-buffer-lower-latency-more-dropouts #latency-and-dropouts
  - audio-midi-setup/recording-latency-is-compensated-automatically #recording-latency
  - audio-midi-setup/input-check #input-check
  - audio-midi-setup/refresh-devices-after-plugging-in #plugging-in-a-new-device
  - audio-midi-setup/unplugged-output-automatic-fallback-and-return #when-the-output-device-disappears
  - audio-midi-setup/device-closed-status-when-another-engine-plays #device-closed
  - audio-midi-setup/engine-not-responding-notice #the-engine-is-not-responding
  - audio-midi-setup/engine-log-desktop #the-engine-log
---

Cette page explique comment faire entrer et sortir le son : quel périphérique joue, lequel enregistre, et comment arbitrer entre latence et stabilité. L’essentiel concerne l’application de bureau. Sur le web, ce sont le navigateur et votre système qui choisissent les périphériques.

## les réglages audio [[desktop]] {#the-audio-settings}

Cliquez sur l’engrenage en haut à droite de la fenêtre pour ouvrir les **réglages**, puis allez dans l’onglet **audio**. Vous pouvez aussi l’ouvrir depuis la palette de commandes ([[action:palette.open]]) avec **réglages audio…**.

| réglage | ce que c’est |
|---|---|
| **pilote** | le système audio à utiliser, par exemple ASIO ou WASAPI sous Windows. Il n’apparaît que si votre ordinateur en a plusieurs. |
| **périphérique de sortie** | là où vous entendez Ethereal. Le périphérique par défaut du système est marqué **(par défaut du système)**. |
| **périphérique d’entrée** | ce que vous enregistrez. Il est sur **aucun (pas d’enregistrement)** tant que vous n’en choisissez pas un. Chaque périphérique affiche son nombre de canaux. |
| **fréquence d’échantillonnage** | la fréquence à laquelle tourne le périphérique, par exemple 48000 Hz. |
| **taille du buffer** | le nombre d’échantillons que le périphérique traite à la fois (voir [latence et coupures](#latency-and-dropouts)). |

Chaque changement s’applique immédiatement et est mémorisé pour le prochain démarrage de l’application. Il n’y a pas de bouton appliquer.

### dans le navigateur [[web]] {#in-the-browser}

Sur le web, l’onglet audio ne peut ni lister ni changer les périphériques. Il indique que c’est votre système qui choisit les périphériques. Pour utiliser d’autres enceintes ou un autre casque, changez la sortie dans votre système d’exploitation ou votre navigateur. Enregistrer depuis une entrée audio demande l’application de bureau (voir [l’enregistrement](/docs/making-music/recording)).

## latence et coupures {#latency-and-dropouts}

Sous les réglages, une ligne d’état montre ce que fait réellement le périphérique : s’il tourne, le pilote, la fréquence d’échantillonnage, la taille du buffer, la latence de sortie et d’entrée en millisecondes, et le nombre de coupures depuis son démarrage (quand il y en a eu).

La taille du buffer est un compromis :

- Un **buffer plus petit** donne une latence plus faible : ce que vous jouez ou chantez revient plus vite. C’est important quand vous enregistrez ou jouez d’un instrument en direct. Mais l’ordinateur a moins de temps pour calculer chaque bloc, et vous avez plus de **coupures** : des clics et des craquements.
- Un **buffer plus grand** est plus stable mais ajoute du retard. Il convient au mixage et à l’édition.

Si vous entendez des craquements, augmentez la taille du buffer. Si le jeu en direct vous semble en retard, diminuez-la.

### ce que vous demandez et ce que le périphérique fait tourner {#what-you-ask-for-and-what-the-device-runs}

Le sélecteur de buffer propose des tailles de 32 à 2048 échantillons, limitées à ce que votre périphérique accepte. Il affiche la taille à laquelle tourne réellement le périphérique, qui n’est pas toujours celle que vous avez demandée :

- Si le périphérique tourne à une autre taille, une remarque sous les réglages l’indique, par exemple « le périphérique tourne à 256 échantillons (128 demandés) ».
- Avec WASAPI en mode partagé sous Windows, le sélecteur est désactivé : c’est Windows qui fixe la taille du buffer. Utilisez un pilote ASIO pour la choisir vous-même.

### latence d’enregistrement {#recording-latency}

Quand vous enregistrez, Ethereal place l’audio et le MIDI enregistrés en fonction de la latence que signale le pilote (entrée plus sortie plus mise en mémoire tampon), pour que les prises s’alignent sur ce que vous avez entendu. Il n’y a pas de réglage de décalage manuel. Si votre pilote signale une latence erronée, les enregistrements peuvent être légèrement en avance ou en retard.

## contrôle de l’entrée {#input-check}

En bas de l’onglet audio, **contrôle de l’entrée** affiche un indicateur de niveau pour chaque piste armée. Servez-vous-en pour vérifier que votre interface fonctionne avant d’enregistrer. Il faut trois choses :

1. un périphérique d’entrée choisi plus haut,
2. une piste armée (● sur son en-tête),
3. le monitoring de cette piste activé (voir [monitoring](/docs/making-music/recording#monitoring)).

## brancher un nouveau périphérique {#plugging-in-a-new-device}

Si vous branchez une interface ou un casque pendant qu’Ethereal est ouvert et qu’ils n’apparaissent pas dans les listes, cliquez sur le bouton d’actualisation en haut de l’onglet audio.

## contrôleurs midi {#midi-controllers}

Il n’y a pas de liste de périphériques MIDI à configurer. Les claviers et contrôleurs MIDI branchés à votre ordinateur sont utilisés tels quels : une piste MIDI écoute **tous les ports** par défaut. Pour qu’une piste n’écoute qu’un seul contrôleur ou un seul canal, voir [filtres d’entrée MIDI](/docs/making-music/recording#midi-input-filters). Pour piloter des boutons avec un contrôleur, voir [l’apprentissage MIDI](/docs/advanced/midi-learn).

Sur le web, l’entrée MIDI matérielle n’est pas disponible : utilisez le [clavier MIDI de l’ordinateur](/docs/making-music/computer-midi-keyboard).

## dépannage {#troubleshooting}

### quand le périphérique de sortie disparaît {#when-the-output-device-disappears}

Si votre périphérique de sortie disparaît pendant qu’Ethereal joue (une interface USB débranchée, un casque Bluetooth éteint), un avertissement nomme le périphérique et Ethereal passe à la sortie par défaut : vous pouvez donc soudain entendre les haut-parleurs de l’ordinateur portable. Si cela échoue aussi, le moteur continue de tourner sans son. Quand le périphérique revient, Ethereal y repasse et vous indique qu’il « est de retour ».

Vos réglages enregistrés ne changent pas : le repli est temporaire. Choisir un périphérique dans l’onglet audio y met fin.

### périphérique fermé {#device-closed}

Quand votre session joue à travers le moteur d’un autre ordinateur (**jouer avec** dans une [session en direct](/docs/sharing/live-sessions)), la ligne d’état indique « périphérique fermé : aucun moteur local ne joue ». C’est normal : votre périphérique audio est libéré tant que vous ne l’utilisez pas. Il se rouvre avec les mêmes réglages quand vous revenez.

### le moteur ne répond pas {#the-engine-is-not-responding}

Si le moteur audio se bloque, par exemple pendant qu’un plug-in est occupé à ouvrir son éditeur, un avertissement indique « le moteur ne répond pas », avec la raison quand elle est connue. L’audio continue de jouer, et ce que vous cliquez est mis en file d’attente et s’exécute quand le moteur se rétablit. Ne cliquez pas encore et encore sur la même chose : chaque clic s’appliquera plus tard. Quand c’est fini, un message indique combien de temps cela a duré.

### le journal du moteur [[desktop]] {#the-engine-log}

Dans **réglages › avancé**, **ouvrir le journal du moteur** ouvre une fenêtre de terminal qui suit le journal du moteur en direct : avertissements, problèmes de plug-ins et raisons pour lesquelles le moteur a cessé de répondre. C’est utile quand vous [signalez un problème](/docs/reference/reporting-issues).

## et ensuite {#where-next}

- [Enregistrement](/docs/making-music/recording) : armer une piste et enregistrer.
- [Dépannage](/docs/reference/troubleshooting) : d’autres solutions pour l’audio, les plug-ins et le partage.
