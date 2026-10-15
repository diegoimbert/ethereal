---
title: projets d’autres daw
order: 20
summary: Ouvrez des fichiers Ableton Live, FL Studio, REAPER et DAWproject comme projets Ethereal, lisez le rapport d’import et résolvez ce qui manque.
platform: all
audience: everyone
covers:
  - import-daw/which-daw-files-open #which-files-open
  - import-daw/where-to-start-an-import #starting-an-import
  - import-daw/an-import-makes-a-new-project #an-import-makes-a-new-project
  - import-daw/import-is-beta #importing-is-in-beta
  - import-daw/progress-and-cancel #while-it-imports
  - import-daw/reading-the-import-report #the-import-report
  - import-daw/right-click-a-report-line-to-resolve-it #the-import-report
  - import-daw/nothing-resolvable-is-thrown-away #nothing-is-thrown-away
  - import-daw/resolve-missing-devices-samples #resolving-missing-items
  - import-daw/resolve-later-and-getting-back-to-it #resolving-later
  - import-daw/show-when-a-project-opens-switch #resolving-later
  - import-daw/replacing-a-missing-device-carries-values-over #replacing-a-device-or-plugin
  - import-daw/plugin-state-kept-only-for-the-same-plugin #replacing-a-device-or-plugin
  - import-daw/ai-suggestions-for-missing-plugins-optional #ai-suggestions
  - import-daw/finding-missing-audio-files #finding-missing-audio-files
  - import-daw/on-the-web-referenced-audio-comes-in-missing #audio-files-on-the-web
  - import-daw/ableton-live-what-converts #ableton-live
  - import-daw/ableton-live-what-is-lost-or-approximated #ableton-live
  - import-daw/fl-studio-what-converts-and-what-doesn-t #fl-studio
  - import-daw/dawproject-reaper-per-app-gaps #dawproject-and-reaper
  - import-daw/size-limits-on-imports #size-limits
---

Vous avez commencé un morceau dans une autre DAW ? Ethereal peut l’ouvrir comme nouveau projet Ethereal. Cette page présente les fichiers qui s’ouvrent, comment lire le rapport qui suit, et comment corriger ce qui n’est pas passé. Pour envoyer un projet dans l’autre sens, voir [exporter vers d’autres DAW](/docs/import-export/exporting-to-other-daws).

## fichiers qui s’ouvrent {#which-files-open}

| Format | Fichiers | Créés par |
|---|---|---|
| Sets Ableton Live | `.als` | Ableton Live |
| Projets FL Studio | `.flp` | FL Studio |
| Projets REAPER | `.rpp`, `.rpp-bak` | REAPER |
| DAWproject | `.dawproject` | Bitwig Studio, Studio One, Cubase, Nuendo, Cubasis, VST Live, n-Track Studio, Waveform |

DAWproject est un format d’échange ouvert. Si votre DAW sait l’exporter (par exemple **File › Export DAWproject** dans Bitwig Studio, ou **File › Convert To › DAWproject File** dans Studio One), c’est en général la voie la plus fidèle. Logic Pro et Pro Tools ne savent écrire aucun de ces formats.

## lancer un import {#starting-an-import}

Toutes ces méthodes fonctionnent :

- Dans le menu du projet (en haut à gauche), **importer depuis une autre DAW**, puis choisissez la DAW. Chaque choix n’affiche que les fichiers de cette DAW.
- Sur l’écran d’accueil, **importer depuis une autre DAW…**.
- Dans la palette de commandes ([[action:palette.open]]), **importer depuis une autre DAW…**.
- Déposez le fichier n’importe où sur la fenêtre d’Ethereal.

### un import crée un nouveau projet {#an-import-makes-a-new-project}

L’import ne modifie jamais le fichier d’origine et ne fusionne jamais avec le projet ouvert. Ethereal crée un nouveau projet à partir du fichier et bascule dessus à la fin. Si vous êtes dans une [session en direct](/docs/sharing/live-sessions), Ethereal vous demande d’abord de la quitter (**quitter et importer**), depuis le menu ou quand vous déposez le fichier sur la fenêtre.

## l’import est en bêta {#importing-is-in-beta}

La conversion depuis d’autres DAW se fait au mieux : certaines choses peuvent manquer ou être mal importées. Ethereal le signale pendant l’import et dans le rapport. Si un import se passe mal, cliquez sur **envoyer les diagnostics…** dans le rapport pour nous le dire.

## pendant l’import {#while-it-imports}

Une fenêtre affiche l’étape en cours et une barre de progression. **Annuler** arrête l’import et supprime le projet à moitié créé.

## le rapport d’import {#the-import-report}

À la fin de l’import, le nouveau projet s’ouvre et un rapport liste ce qui est arrivé (pistes, clips, notes, etc.), puis jusqu’à trois sections :

- **manquants : gardés pour être résolus** : les plug-ins qui ne sont pas installés, les appareils sans équivalent dans Ethereal, les fichiers audio introuvables. Ils sont dans le projet et vous attendent.
- **importés approximativement** : ce qui est arrivé proche de l’original, mais pas tout à fait identique.
- **non importés** : ce qu’Ethereal n’a pas pu reprendre.

Chaque ligne est étiquetée avec son domaine (pistes, appareils, plug-ins, fichiers audio…) et un nombre. Faites un clic droit sur une ligne de plug-in, de fichier audio ou d’appareil manquant et choisissez **résoudre…** pour corriger cet élément tout de suite. Le rapport garde les 500 premières lignes distinctes, puis indique combien il en reste.

### rien n’est jeté {#nothing-is-thrown-away}

Tout ce qui pourrait être corrigé plus tard reste dans le projet :

- Un **plug-in qui n’est pas installé** garde son son enregistré, ses valeurs et son automation. Il est contourné jusqu’à ce que vous l’installiez ou le remplaciez.
- Un **appareil de l’autre DAW** sans équivalent dans Ethereal (Hybrid Reverb de Live, appareils Max for Live, Polymer de Bitwig…) reste dans la chaîne comme **appareil manquant**, avec ses valeurs et son automation. L’audio le traverse sans changement, et un instrument manquant reste silencieux.
- Un **fichier audio introuvable** reste comme une référence manquante vers laquelle pointent ses clips.

## résoudre les éléments manquants {#resolving-missing-items}

Cliquez sur **terminé** dans le rapport et Ethereal ouvre **résoudre les éléments manquants** (le rapport a aussi les boutons **résoudre les appareils manquants** et **résoudre les échantillons manquants**). La fenêtre liste chaque plug-in, appareil et fichier audio manquant, chacun avec :

- un remplacement proposé,
- le degré de certitude de la correspondance : **même identifiant de plug-in**, **même nom**, **même famille** ou **nom proche**, avec un pourcentage,
- et la raison de cette proposition.

Pour chaque élément, vous pouvez activer la proposition, choisir un autre remplacement dans la liste, ou le laisser tel quel. Cliquez sur **appliquer** pour faire les changements. Ensemble, ils comptent comme une seule étape d’annulation. **tout accepter** applique toutes les propositions d’un coup.

### remplacer un appareil ou un plug-in {#replacing-a-device-or-plugin}

Vous pouvez remplacer un appareil manquant par un appareil intégré du même type ou par n’importe quel plug-in installé. Ses valeurs, son automation, sa modulation et ses assignations MIDI passent aux paramètres du même nom : « DryWet », « Dry/Wet » et « Mix » comptent comme le même. Tout ce qui n’a pas de correspondance est listé dans une notification après l’application.

Pour les plug-ins :

- Remplacé par **le même plug-in** dans une autre version ou un autre format, le plug-in garde son son enregistré (« état du plug-in conservé »).
- Remplacé par **un autre plug-in**, seules les valeurs des paramètres qui correspondent par nom (ou par position, pour le même plug-in) survivent. La fenêtre liste ce qui sera perdu.

### suggestions de l’ia {#ai-suggestions}

Si vous avez configuré un fournisseur d’IA dans [l’assistant IA](/docs/advanced/ai-assistant), la fenêtre peut l’interroger sur les plug-ins sans correspondance sûre. Seuls les détails des plug-ins sont envoyés : noms, éditeurs, formats, identifiants, versions et noms des paramètres enregistrés. **montrer ce qui sera envoyé** vous montre exactement ce qui part, et Ethereal vous demande votre accord une fois. Les suggestions sont marquées **suggestion de l’IA** et ne s’appliquent que si vous les activez.

### retrouver les fichiers audio manquants {#finding-missing-audio-files}

Ethereal cherche un fichier audio manquant par son nom, puis vérifie la fréquence d’échantillonnage, les canaux et la durée (et le contenu du fichier, quand il est connu). Il cherche dans le dossier du projet importé, dans les dossiers `Samples` à côté et au-dessus, et dans votre bibliothèque. Sur le bureau, **chercher dans un dossier…** cherche dans un dossier de votre choix. Vous pouvez aussi choisir le fichier vous-même avec **choisir un fichier…**.

### résoudre plus tard {#resolving-later}

Chaque élément peut attendre. Cliquez sur **résoudre plus tard** et le projet garde les éléments manquants tels quels. Pour y revenir :

- le badge **n manquants** dans la barre du haut,
- **résoudre les appareils et échantillons manquants…** dans le menu du projet ou la palette de commandes,
- le bouton **résoudre…** sur un appareil manquant,
- un clic droit sur un clip ou un appareil manquant, puis **résoudre…**.

La fenêtre s’ouvre aussi d’elle-même quand vous ouvrez un projet qui a des éléments manquants. Désactivez **afficher à l’ouverture d’un projet** dans la fenêtre pour éviter cela.

## les fichiers audio sur le web [[web]] {#audio-files-on-the-web}

Un fichier REAPER ou DAWproject peut pointer vers des fichiers audio situés ailleurs sur votre disque au lieu de les contenir. L’application de bureau les trouve dans les dossiers à côté du projet. Le navigateur ne peut pas lire votre disque : ces clips arrivent donc comme des références manquantes. Reliez-les dans la fenêtre de résolution avec **choisir un fichier…**. L’audio stocké dans le fichier lui-même (DAWproject peut l’intégrer) arrive toujours.

## ce qui est converti depuis chaque daw {#what-converts-from-each-daw}

### ableton live {#ableton-live}

Ce qui passe : les pistes, les groupes, les pistes de retour, le mixeur et les envois, les clips de l’arrangement avec le warp, les fondus, le gain et la transposition, les couloirs de prises, l’automation des pistes et des clips pour les appareils qu’Ethereal sait faire correspondre, et la plupart des appareils fournis avec Live : EQ Eight, Compressor, Gate, Delay, Utility et bien d’autres, plus les Instrument, Audio Effect, MIDI Effect et Drum Racks, Simpler et Sampler, et les plug-ins VST2, VST3 et AU avec leur état. Une piste gelée arrive sous forme de son audio gelé sur une nouvelle piste, l’originale étant gardée et rendue muette.

Ce qui est perdu ou approximé :

- **Les clips de la vue Session** sont disposés scène par scène (avec un marqueur pour chaque scène) seulement quand l’arrangement est vide. Sinon, ils sont laissés de côté.
- **La probabilité des notes et la plage de vélocité** (Live 11 et plus) sont ignorées : les notes jouent à leur propre vélocité.
- **Les enveloppes de gain et de transposition des clips**, et les enveloppes détachées de leur clip, sont supprimées.
- **Arpeggiator, Chord, Scale, Note Length et Random** arrivent comme les appareils d’Ethereal avec leurs réglages par défaut.
- **Les macros des racks** arrivent sans leurs cibles.
- **Un Drum Rack dans un pad** est aplati dans le rack extérieur.
- **Les modes de warp Tones et Texture** deviennent l’étirement d’Ethereal.
- **Les appareils natifs sans équivalent** (Hybrid Reverb, Operator, Max for Live…) sont gardés comme appareils manquants à résoudre.

### fl studio {#fl-studio}

Chaque canal qui joue quelque chose devient une piste. Les clips de pattern de la playlist deviennent des clips MIDI, et les inserts du mixeur dans lesquels jouent les canaux deviennent des pistes de groupe (ou des pistes de retour, quand ils reçoivent des envois). Ce qui est perdu ou approximé :

- Seul **l’arrangement en cours** est importé.
- Quand la **playlist est vide** (un projet fait seulement dans le channel rack et le piano roll), les patterns sont posés les uns après les autres dans leur ordre, à partir de la mesure 1, chacun aussi long que ses notes arrondies à la mesure, avec un marqueur pour chaque pattern. Chaque canal a sa propre piste, et chaque pattern un clip avec son nom et sa couleur sur les canaux qu’il joue.
- **Les clips d’automation** n’arrivent que s’ils automatisent le volume d’un canal.
- Les réglages **d’étirement et de hauteur** de FL sur l’audio sont ignorés : l’audio joue à sa propre vitesse.
- **Les effets natifs** de FL deviennent l’appareil intégré le plus proche, avec ses réglages par défaut. Les plug-ins natifs sans équivalent sont gardés comme appareils manquants.
- **Les courbes de volume et de fader** sont approximées.

### dawproject et reaper {#dawproject-and-reaper}

Les deux formats arrivent bien : pistes, groupes, retours et envois, clips MIDI et audio, changements de tempo et de signature rythmique, marqueurs, plug-ins avec leur état et la plupart de l’automation. Ce qui est perdu ou approximé :

- **Les formes de fondu** deviennent linéaires, **le panoramique des envois** et **le solo du master** sont supprimés.
- **Les pistes VCA** deviennent des groupes. **Les scènes du lanceur de clips** et **la vidéo** sont laissées de côté.
- **Waveform** n’exporte ni l’automation ni les envois : ils ne sont donc pas dans le fichier.
- **Les effets propres à Studio One** n’arrivent que comme EQ, compresseur, gate et limiteur génériques.
- **Les appareils propres à Bitwig** (Polymer, Grid…) sont gardés comme appareils manquants.
- Depuis **REAPER** : l’automation des paramètres VST3 et CLAP, les effets JS et LV2, les plug-ins AU et les régions ne sont pas importés.

## limites de taille {#size-limits}

Les très gros projets sont tronqués, et le rapport le signale : un import s’arrête à 4 millions de notes ou 1 million de clips, et un projet ne peut pas dépasser 512 Mo. Un fichier audio dans un DAWproject peut aller jusqu’à 2 Go sur le bureau et 1 Go dans le navigateur.

## et ensuite {#where-next}

- [Les plug-ins](/docs/sounds-and-plugins/plugins) : installer et rescanner les plug-ins.
- [Exporter vers d’autres DAW](/docs/import-export/exporting-to-other-daws) : renvoyez un projet.
- [Importer de l’audio](/docs/import-export/importing-audio) : seulement les fichiers audio.
