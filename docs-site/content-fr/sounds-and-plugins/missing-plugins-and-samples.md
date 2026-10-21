---
title: plug-ins et échantillons manquants
order: 30
summary: Remplacez les plug-ins, appareils et fichiers audio qu’utilise un projet et qui ne sont pas sur cet ordinateur, tout de suite ou plus tard.
platform: all
audience: everyone
covers:
  - import-daw/resolve-missing-devices-samples #resolving-missing-items
  - import-daw/resolve-later-and-getting-back-to-it #resolving-later
  - import-daw/show-when-a-project-opens-switch #resolving-later
  - import-daw/replacing-a-missing-device-carries-values-over #replacing-a-device-or-plugin
  - import-daw/plugin-state-kept-only-for-the-same-plugin #replacing-a-device-or-plugin
  - import-daw/ai-suggestions-for-missing-plugins-optional #ai-suggestions
  - import-daw/finding-missing-audio-files #finding-missing-audio-files
---

Un projet peut utiliser des éléments absents de l’ordinateur sur lequel vous l’ouvrez : un plug-in que vous n’avez pas installé, ou un fichier audio qui a été déplacé. Ethereal ne les jette jamais. Un plug-in manquant reste dans sa chaîne, contourné, avec son son enregistré, et un fichier audio manquant garde son clip. Cette page montre comment les remplacer, ou les laisser pour plus tard. Pour corriger un seul échantillon manquant, [le relier](/docs/basics/moving-projects#missing-samples) est souvent plus rapide.

## résoudre les éléments manquants {#resolving-missing-items}

**résoudre les éléments manquants** liste chaque plug-in, appareil et fichier audio manquant, chacun avec :

- un remplacement proposé,
- le degré de certitude de la correspondance : **même identifiant de plug-in**, **même nom**, **même famille** ou **nom proche**, avec un pourcentage,
- et la raison de cette proposition.

Pour chaque élément, vous pouvez activer la proposition, choisir un autre remplacement dans la liste, ou le laisser tel quel. Cliquez sur **appliquer** pour faire les changements. Ensemble, ils comptent comme une seule étape d’annulation. **tout accepter** applique toutes les propositions d’un coup.

### remplacer un appareil ou un plug-in {#replacing-a-device-or-plugin}

Vous pouvez remplacer un appareil manquant par un appareil intégré du même type ou par n’importe quel plug-in installé. Ses valeurs, son automation, sa modulation et ses assignations MIDI passent aux paramètres du même nom : « DryWet », « Dry/Wet » et « Mix » comptent comme le même. Tout ce qui n’a pas de correspondance est listé dans une notification après l’application.

Pour les plug-ins :

- Remplacé par **le même plug-in** dans une autre version ou un autre format, le plug-in garde son son enregistré (« état du plug-in conservé »).
- Remplacé par **un autre plug-in**, seules les valeurs des paramètres qui correspondent par nom (ou par position, pour le même plug-in) survivent. La fenêtre liste ce qui sera perdu.

Pour échanger un appareil qui n’est pas manquant, faites un clic droit sur son en-tête et choisissez **remplacer l’appareil…** (voir [plug-ins](/docs/sounds-and-plugins/plugins#missing-plugins)).

### suggestions de l’ia {#ai-suggestions}

Si vous avez configuré un fournisseur d’IA dans [l’assistant IA](/docs/advanced/ai-assistant), la fenêtre peut l’interroger sur les plug-ins sans correspondance sûre. Seuls les détails des plug-ins sont envoyés : noms, éditeurs, formats, identifiants, versions et noms des paramètres enregistrés. **montrer ce qui sera envoyé** vous montre exactement ce qui part, et Ethereal vous demande votre accord une fois. Les suggestions sont marquées **suggestion de l’IA** et ne s’appliquent que si vous les activez.

### retrouver les fichiers audio manquants {#finding-missing-audio-files}

Ethereal cherche un fichier audio manquant par son nom, puis vérifie la fréquence d’échantillonnage, les canaux et la durée (et le contenu du fichier, quand il est connu). Sur le bureau, **chercher dans un dossier…** cherche dans un dossier de votre choix. Vous pouvez aussi choisir le fichier vous-même avec **choisir un fichier…**.

## résoudre plus tard {#resolving-later}

Chaque élément peut attendre. Cliquez sur **résoudre plus tard** et le projet garde les éléments manquants tels quels. Pour y revenir :

- le badge **n manquants** dans la barre d’état,
- **résoudre les appareils et échantillons manquants…** dans le menu du projet ou la palette de commandes,
- le bouton **résoudre…** sur un appareil manquant,
- un clic droit sur un clip ou un appareil manquant, puis **résoudre…**.

La fenêtre s’ouvre aussi d’elle-même quand vous ouvrez un projet qui a des éléments manquants. Désactivez **afficher à l’ouverture d’un projet** dans la fenêtre pour éviter cela.

## et ensuite {#where-next}

- [Plug-ins](/docs/sounds-and-plugins/plugins) : installer et analyser de nouveau les plug-ins.
- [Déplacer des projets entre ordinateurs](/docs/basics/moving-projects) : emportez un projet et ses échantillons.
- [Le mode sans échec](/docs/advanced/safe-mode) : ouvrez un projet sans ses plug-ins.
