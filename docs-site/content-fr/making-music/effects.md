---
title: effets
order: 70
summary: Ce que fait chaque effet audio et MIDI intégré, et les réglages qui comptent le plus.
platform: all
audience: everyone
covers:
  - devices/compressor #compressor
  - devices/eq #eq
  - devices/delay #delay
  - devices/grain-delay #grain-delay
  - devices/reverb #reverb
  - devices/convolution-reverb #convolution-reverb
  - devices/limiter #limiter
  - devices/utility #utility
  - devices/color-effects-saturator-bitcrusher-auto-filter #color-saturator-bitcrusher-auto-filter
  - devices/modulation-effects-chorus-phaser-flanger-tremolo #modulation-chorus-phaser-flanger-tremolo
  - devices/shifter #shifter
  - devices/dynamics-gate-multiband-compressor-transient-shaper #dynamics-gate-multiband-compressor-transient-shaper
  - devices/analysis-spectrum-and-tuner #analysis-spectrum-and-tuner
  - devices/midi-effects-arpeggiator-chord-scale-note-length-velocity-ra #midi-effects
  - devices/external-instrument-external-audio-effect #external-audio-effect
---

Les effets transforment un son. Les **effets audio** se placent après l’instrument (ou sur une piste audio) et traitent l’audio. Les **effets MIDI** se placent avant l’instrument sur une piste MIDI et modifient les notes qu’il reçoit. Ajoutez-les avec les boutons d’ajout de la chaîne (voir [appareils et presets](/docs/making-music/devices-and-presets#adding-a-device)). Chaque effet a des presets d’usine, un bon moyen de découvrir ce qu’il sait faire.

La plupart des effets audio ont un réglage **mix** (la part d’effet que vous entendez par rapport au son d’origine) et un niveau de **sortie**.

## l’essentiel {#the-essentials}

Ce sont les effets que la plupart des morceaux utilisent.

### eq {#eq}

Un égaliseur 8 bandes pour couper ou accentuer des fréquences.

- Chaque bande est un coupe-bas, un shelf grave, une cloche, un coupe-bande, un shelf aigu ou un coupe-haut. Les bandes 2 à 7 démarrent activées, les bandes 1 et 8 (un coupe-bas et un coupe-haut) désactivées.
- **Sur la courbe** : faites glisser la poignée d’une bande pour changer sa fréquence (gauche et droite) et son gain (haut et bas). Faites-la glisser avec [[key:Alt]] ou tournez la molette dessus pour changer sa largeur (Q). [[key:Shift]] rend tout glissement plus fin. Double-cliquez sur une poignée pour activer ou désactiver la bande ; faites un clic droit dessus pour changer son type ou la réinitialiser.
- Une poignée qui a le focus répond aux flèches : gauche et droite pour la fréquence, haut et bas pour le gain, [[key:Alt]] avec haut et bas pour le Q.
- **canal** : traiter les deux canaux, seulement le gauche ou le droit, ou seulement le mid ou le side d’un signal stéréo.
- Le spectre en direct derrière la courbe montre ce qui entre et ce qui sort.

### compresseur {#compressor}

Égalise le volume : quand le son dépasse le **seuil**, il est baissé selon le **ratio**.

- **attaque** et **relâchement** : la vitesse à laquelle il réagit et relâche.
- **knee** : un knee doux commence à compresser progressivement autour du seuil.
- **gain de compensation** remonte le résultat ; **compensation auto** le fait pour vous.
- **mix** : mélangez le son non compressé (compression parallèle).
- **détecteur** : **crête** réagit à chaque crête, **RMS** au niveau moyen, et **expansion** le transforme en expandeur vers le bas qui rend les passages calmes plus calmes.
- l’**anticipation** (jusqu’à 10 ms) attrape les transitoires rapides, et ajoute autant de latence, qu’Ethereal compense.
- Il peut être déclenché par une autre piste (voir [sidechain](/docs/making-music/modulation-and-sidechain#sidechain)). Le **HPF du sidechain** filtre alors le grave du signal de déclenchement, pour que le sub d’une grosse caisse ne le déclenche pas trop ; la **pente du sidechain** rend ce filtre doux (6 dB/oct) ou plus raide (12 dB/oct).

### réverbe {#reverb}

Une réverbe algorithmique : le son d’une pièce ou d’une salle.

- **pré-délai** : le temps avant le début de la réverbe. **taille** et **déclin** (0,2 à 20 secondes) : la taille de l’espace et la durée de la queue.
- **amortissement** assombrit la queue, **diffusion** la rend plus douce, **largeur** règle sa taille stéréo.
- **coupe-bas** et **coupe-haut** filtrent ce qui entre dans la réverbe.
- **geler** maintient la queue indéfiniment.
- **shimmer** transpose la queue à l’intérieur de la réverbe, si bien que chaque écho monte encore : **quantité** règle la part transposée, **hauteur** l’intervalle en demi-tons (+12 une octave, +7 une quinte, -12 descend), **timbre** assombrit le son transposé. Préréglages : *shimmer hall*, *octave cloud*.
- **mix** et **sortie**. La sortie règle le niveau de la réverbe elle-même, pas du son sec.

### delay {#delay}

Des échos.

- **temps** en millisecondes (jusqu’à 2 secondes), ou activez **sync** pour choisir une valeur de note (de 1/32 à la ronde, valeurs pointées et triolets compris) qui suit le tempo.
- **feedback** : le nombre d’échos. **mix** : leur volume.
- **ping-pong** : les échos rebondissent entre la gauche et la droite (l’entrée est d’abord sommée en mono).
- **coupe-bas** et **coupe-haut** agissent dans la boucle de feedback, donc chaque écho est plus sombre ou plus fin que le précédent.
- **vitesse de mod** et **profondeur de mod** font onduler le temps de delay, comme une bande.
- **geler** boucle ce qui est dans le delay à cet instant, sans prendre de nouvelle entrée.

### délai granulaire {#grain-delay}

Des échos découpés en minuscules grains qui se chevauchent, que vous pouvez transposer et disperser.

- **hauteur** transpose les grains (de -36 à +12 demi-tons) ; avec du **feedback**, chaque écho est transposé à nouveau, donc les répétitions montent ou descendent.
- **fréquence** règle combien de grains démarrent par seconde, et donc leur taille : des valeurs basses donnent des grains longs et doux (un transpositeur simple), des valeurs hautes une texture granuleuse qui bourdonne. Faites glisser le pavé : en travers pour la **fréquence**, vers le haut pour la **hauteur**.
- **dispersion** éparpille le départ de chaque grain dans le temps (un peu étale le son, beaucoup le casse et l’élargit) ; **hauteur aléatoire** désaccorde chaque grain au hasard.
- **temps** en millisecondes, ou activez **sync** pour une valeur de note qui suit le tempo. **mix** mélange le son sec.

### limiteur {#limiter}

Un limiteur brickwall, en général en dernier sur la piste master : rien ne dépasse le **plafond**.

- **gain** pousse l’entrée dedans, pour rendre le son plus fort. **relâchement** règle la vitesse à laquelle il revient.
- Il anticipe toujours de 5 ms (cette latence est compensée). Il attrape aussi les crêtes réelles (true peak, les crêtes entre les échantillons qu’un convertisseur ou un encodeur de streaming reconstruit), donc le plafond tient après l’export.

### utilitaire {#utility}

De petites corrections bien utiles : **gain** (±36 dB), **panoramique**, **largeur** stéréo (0 à 200 %) ou un équilibre mid/side, **inverser** le canal gauche ou droit, **mono**, **basses mono** sous une fréquence de votre choix, un choix de **canal** (stéréo, gauche, droite ou inversés), un **filtre DC** et **muet**. Ses vumètres affichent le niveau de sortie.

## espace {#space}

### réverbe à convolution {#convolution-reverb}

Une réverbe faite à partir de l’enregistrement d’un espace réel (une réponse impulsionnelle, ou IR).

- Choisissez l’une des 6 IR d’usine (petite pièce, chambre, plaque, salle de concert, cathédrale, ambiance), ou déposez-y votre propre fichier audio (seules les 10 premières secondes sont utilisées).
- **mix** démarre à 30 %. **déclin** raccourcit l’IR et **taille** l’étire. **inverser** la joue à l’envers.
- **pré-délai**, **coupe-bas**, **coupe-haut**, **largeur** et **gain**.
- Dans sa fenêtre, faites glisser les repères de pré-délai et de déclin directement sur l’image de l’IR.

## couleur : saturateur, bitcrusher, filtre auto {#color-saturator-bitcrusher-auto-filter}

- **saturateur** : ajoute de la chaleur ou de la distorsion. Choisissez une **courbe** (douce, dure, lampe, bande, repli, redressement), puis réglez **drive**, **biais** et **timbre**. **gain auto** (activé par défaut) garde le niveau stable quand vous ajoutez du drive. Le **suréchantillonnage** le rend plus propre. Il ajoute une petite latence fixe.
- **bitcrusher** : du grain numérique lo-fi. **bits** baisse la résolution, **vitesse** la fréquence d’échantillonnage ; **jitter** et **dither** le rendent plus rugueux ou plus doux.
- **érosion** : une usure numérique qui suit le son. Elle fait trembler un délai très court avec un **sinus** (des bandes latérales métalliques) ou avec du **bruit** (grésillement et souffle ; le **bruit large** diffère de chaque côté). Faites glisser le pavé : en travers pour la **fréquence**, vers le haut pour la **quantité**. **largeur** règle la largeur de la bande de bruit. Les sons brillants s’érodent en premier, et le silence reste silencieux. Elle ajoute une toute petite latence fixe.
- **pédale** : une pédale de guitare. Choisissez un **type** : **overdrive** (chaud et doux, il garde des graves ronds), **distorsion** (serrée et agressive) ou **fuzz** (instable : la fin de chaque note crachote). **gain** règle la force de l’attaque ; même à 0 % il colore le son, et le niveau reste à peu près le même quand vous le montez, alors réglez le volume avec **sortie**. **graves**, **médiums** et **aigus** sculptent le timbre (**fréq. des médiums** choisit 500 hz, 1 khz ou 2 khz), **sub** ajoute du poids sous 250 hz, et **mix** mélange le son sec. Elle tourne suréchantillonnée, ce qui ajoute une petite latence fixe.
- **filtre auto** : un filtre (7 types) qui bouge tout seul. Son **enveloppe** suit le niveau du son (ou d’un sidechain, s’il y en a un) pour ouvrir le filtre ; son **LFO** le balaie en cycle, libre ou synchronisé au tempo, avec un décalage de **phase stéréo** entre la gauche et la droite. Il tourne suréchantillonné, donc le **drive** reste propre ; cela ajoute une petite latence fixe.

## modulation : chorus, phaser, flanger, trémolo {#modulation-chorus-phaser-flanger-tremolo}

Ces effets font bouger le son avec un LFO. La plupart ont **sync** pour caler leur vitesse sur le tempo ; un effet synchronisé suit la position dans le morceau pendant la lecture. Le chorus, le phaser et le flanger démarrent à 50 % de mix.

- **chorus** : épaissit un son avec des copies légèrement désaccordées. Ses modes sont chorus, ensemble et vibrato.
- **phaser** : un son creux qui balaie, avec 2 à 12 étages.
- **flanger** : un balayage façon avion à réaction. **passage par zéro** le rend plus spectaculaire.
- **trémolo** : fait pulser le volume, ou avec **panoramique auto**, déplace le son de gauche à droite.

### décaleur {#shifter}

Déplace la hauteur ou les fréquences d’un son. Choisissez un **mode** :

- **hauteur** transpose de **hauteur** (de -24 à +24 demi-tons) et de **désaccord** (en cents). **fenêtre** règle la longueur des tranches qui le composent : plus longue pour les notes graves et les nappes, plus courte pour les notes aiguës et les batteries.
- **fréquence** déplace chaque partiel du même nombre de hertz vers le haut ou vers le bas (**décalage**, plus **fin**). Les harmoniques ne sont plus harmoniques : quelques hertz contre le son sec tournoient comme un phaser sans fin, des centaines de hertz sonnent métalliques, comme des cloches.
- **anneau** multiplie le son par une **porteuse** (plus **fin**) : aux fréquences audio, des bandes latérales robotiques et métalliques ; à quelques hertz sous un peu de son sec, un trémolo haché. **drive** distord le résultat (de la **quantité**).
- le **lfo** déplace le décalage tout seul : **profondeur du lfo** en hertz (modes fréquence et anneau) ou **hauteur du lfo** en demi-tons (mode hauteur). Choisissez une **forme d’onde** (sinus, triangles, dents de scie, rectangle avec son **rapport cyclique**, aléatoire lisse ou en escalier), une **vitesse** en hertz ou **synchro** pour suivre le tempo. **stéréo** décale le côté droit d’une **phase**, ou le fait tourner plus vite (**rotation**) ; **décalage initial** déplace le point de départ, et **largeur** écarte l’aléatoire en escalier entre les côtés.
- le **suiveur** d’enveloppe le fait suivre le niveau de l’entrée : **profondeur d’env.** en hertz, **hauteur d’env.** en demi-tons (les valeurs négatives descendent), avec son **attaque** et son **relâchement**.
- le **delay** vient après le décalage : un **temps du delay** en millisecondes, ou une **division du delay** avec **synchro du delay**. Son **feedback** repasse par le décaleur, donc chaque répétition est décalée à nouveau (des spirales en mode fréquence, des cascades d’octaves en mode hauteur) ; **timbre** assombrit les répétitions.
- **large** inverse le canal droit (le désaccord ou le décalage opposé ; en mode anneau la porteuse est décalée d’un quart de cycle, donc le trémolo alterne entre les côtés). **mix** mélange le son sec.

## dynamique : gate, compresseur multibande, transient shaper {#dynamics-gate-multiband-compressor-transient-shaper}

- **gate** : coupe le son quand il passe sous le **seuil** (contre le bruit ou la repisse de batterie). **mode** peut en faire un expandeur plus doux ; **inverser** l’inverse ; **maintien**, **anticipation** et une entrée sidechain servent à un gate précis.
- **compresseur multibande** : trois compresseurs, un pour les graves, les médiums et les aigus, séparés par deux fréquences de coupure réglables. Chaque bande peut être mise en solo ou contournée. Un sidechain déclenche les trois bandes.
- **transient shaper** : accentue ou réduit l’**attaque** (le coup) et le **maintien** (le corps) des sons, indépendamment de leur niveau. **écrêtage** empêche la sortie de dépasser.

## analyse : spectre et accordeur {#analysis-spectrum-and-tuner}

Ceux-ci ne changent pas le son : l’audio passe sans être modifié. Ouvrez leur fenêtre pour voir ce qu’ils affichent.

- **spectre** : un analyseur de fréquences, avec taille de bloc, moyennage, canal (mid et side compris), plage d’affichage, pente et maintien des crêtes.
- **accordeur** : affiche la hauteur de ce que vous jouez. La **référence** va de 415 à 466 Hz (440 par défaut). **couper la sortie** rend la piste muette pendant que vous vous accordez.

## effets midi {#midi-effects}

Les effets MIDI se placent sur les pistes MIDI, avant l’instrument.

| effet | ce qu’il fait |
|---|---|
| **arpégiateur** | joue les notes que vous tenez l’une après l’autre : 9 styles (montant, descendant, montant-descendant, convergent, dans l’ordre joué, aléatoire, accord…), une vitesse, une durée de gate, des octaves et l’écart entre elles, du swing, **maintien** et redéclenchement. Il continue de tourner quand le transport est à l’arrêt, pour que vous puissiez en jouer en direct. |
| **accord** | ajoute jusqu’à 6 notes supplémentaires à intervalles fixes à chaque note, chacune avec sa propre vélocité, et peut les **égrener** |
| **gamme** | ramène chaque note dans une gamme : celle de la piste, celle du projet, ou une gamme personnalisée. Il peut arrondir à la note la plus proche, vers le haut ou vers le bas. |
| **durée de note** | règle la durée de chaque note, en temps ou synchronisée au tempo |
| **vélocité** | remodèle les vélocités avec une courbe, une plage de sortie et un peu d’aléatoire |
| **aléatoire** | change au hasard les hauteurs (éventuellement dans la gamme), la vélocité, le placement et la durée. Il dépend d’une **graine**, donc la même graine donne le même résultat à chaque lecture ou export. |
| **hauteur** | transpose les notes, et peut les bloquer, les replier ou les limiter à une plage de touches |
| **bloqueur de notes** | supprime toutes les notes, donc rien de ce qui suit ne joue : comme couper le MIDI en entrée, en gardant la chaîne. Les CC, le pitch bend et l’aftertouch passent toujours, sauf si **tout bloquer** est activé. Les notes tenues quand vous l’ajoutez ou l’activez s’arrêtent, et le désactiver ne les rejoue pas : elles reviennent à la note suivante. |

## effet audio externe {#external-audio-effect}

Envoie l’audio d’une piste vers un effet matériel et le fait revenir [[desktop]].

- Dans sa fenêtre, choisissez les **sorties audio** où l’entrée de l’effet est branchée et les **entrées audio** par lesquelles il revient.
- **gain d’envoi**, **gain de retour**, **mix** et **inverser la phase**.
- **mesurer** chronomètre l’aller-retour et règle la **latence**, pour que le matériel reste en place.
- Les ports absents restent sélectionnés mais sont marqués comme absents, et l’appareil reste muet jusqu’à leur retour. Dans l’application web, le routage ne peut pas être modifié.

## et ensuite {#where-next}

- [Modulation et sidechain](/docs/making-music/modulation-and-sidechain) : faites baisser une basse sous la grosse caisse, ou bougez des paramètres avec un LFO.
- [Racks et macros](/docs/making-music/racks-and-macros) : combinez plusieurs effets en un seul appareil.
- [Le mixage](/docs/making-music/mixing) : volume, panoramique et pistes de retour pour partager des réverbes.
