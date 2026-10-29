---
title: effets
order: 70
summary: Ce que fait chaque effet audio et MIDI intégré, et les réglages qui comptent le plus.
platform: all
audience: everyone
covers:
  - devices/compressor #compressor
  - devices/eq #eq
  - devices/parametric-eq #parametric-eq
  - devices/delay #delay
  - devices/grain-delay #grain-delay
  - devices/reverb #reverb
  - devices/convolution-reverb #convolution-reverb
  - devices/limiter #limiter
  - devices/maximizer #maximizer
  - devices/shaper #shaper
  - devices/pitch-correction #pitch-correction
  - devices/adaptive-eq #adaptive-eq
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

Un égaliseur qui se dessine : coupez ou accentuez des fréquences en plaçant des points sur sa courbe. Il démarre à plat (une ligne droite, aucune bande : le son passe intact) et peut contenir jusqu’à 24 bandes. La fenêtre se redimensionne : la courbe grandit avec elle.

- **Ajouter une bande** : double-cliquez sur la ligne. Une cloche y apparaît ; gardez le second clic enfoncé et bougez pour la faire glisser tout de suite.
- **Faites glisser un point** pour changer sa fréquence (gauche et droite) et son gain (haut et bas). Les bandes sans gain (coupe-bas, coupe-haut, coupe-bande, passe-bande) changent leur Q quand on les fait glisser vers le haut ou le bas. Faites glisser avec [[key:Alt]] ou tournez la molette sur un point pour changer sa largeur ou sa résonance (Q) ; [[key:Shift]] rend tout glissement plus fin.
- **Supprimer une bande** : double-cliquez sur son point, ou appuyez sur [[key:Delete]] sur un point qui a le focus.
- **Cliquez sur un point** pour ouvrir son panneau : type, fréquence, gain, Q, pente, activation et suppression. **Faites un clic droit** dessus pour son type, sa pente et son activation ; un clic droit sur le fond supprime toutes les bandes.
- **Types** : cloche, plateau grave, coupe-bas, plateau aigu, coupe-haut, coupe-bande, passe-bande, plateau incliné (les graves baissent et les aigus montent autour de la fréquence, ou l’inverse) et inclinaison globale (une pente régulière sur toute la plage, 0 dB à la fréquence). Les coupe-bas, coupe-haut et le passe-bande ont une **pente** de 6 à 96 dB/oct ; leur Q est la résonance à la coupure (0,71 est plat).
- Le spectre en direct derrière la courbe montre ce qui entre et ce qui sort. Le bouton au-dessus change la plage affichée (±6, 12, 24 ou 30 dB) ; **sortie** règle le niveau.
- Chaque bande est faite de paramètres ordinaires : vous pouvez les automatiser, les associer à un contrôleur MIDI ou à une macro, et annuler tout changement. Il n’ajoute aucune latence.

### eq paramétrique {#parametric-eq}

L’ancien égaliseur 8 bandes, gardé pour sa disposition fixe : chaque bande est toujours là, avec sa propre rangée de boutons.

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

### maximiseur {#maximizer}

Un limiteur de mastering pour la sonie : baissez le **seuil** et le son devient d’autant plus fort, sans que rien ne dépasse le **plafond** (crêtes réelles comprises, -1 dB par défaut).

- **attaque** règle combien de temps avant une crête le gain commence à baisser (dans les 5 ms d’anticipation, compensées). **relâchement** règle la vitesse à laquelle il revient ; avec **relâchement auto**, il revient vite après une crête isolée et lentement après un passage fort, ce qui pompe moins.
- **liaison stéréo** à 100 % baisse les deux côtés ensemble ; plus bas, une crête d’un côté baisse moins l’autre, ce qui garde un mix large. **écrêtage** arrondit les crêtes les plus rapides avant le limiteur, pour un son plus dense et plus fort.
- **niveau égalisé** baisse la sortie du gain apporté par le seuil, pour entendre ce que fait la limitation sans que la sonie vous trompe. Désactivez-le avant d’exporter.
- Le graphe montre les 4 dernières secondes : le niveau d’entrée, la sortie et la réduction de gain qui descend du haut, avec la ligne du plafond. Ses valeurs affichent la réduction actuelle, la plus forte et la crête réelle de la sortie depuis la lecture.

### façonnage {#shaper}

Trois formes dessinées qui se répètent en rythme avec le morceau : une pour le **volume**, une pour la **coupure** d’un filtre et une pour le **panoramique**. Chaque piste a sa propre **vitesse** (1/64 à 8 mesures), sa **profondeur** et son interrupteur ; le volume est actif par défaut (un pompage façon sidechain), la coupure et le panoramique sont éteints.

- Dessinez dans une piste comme dans le [modeleur de volume](#volume-shaper) : double-cliquez pour ajouter ou supprimer un point, faites glisser les points (ils s’alignent sur la grille, maintenez alt pour ne pas aligner), alt-glissez un segment pour le courber, clic droit pour un segment en escalier, linéaire ou lissé. Une piste éteinte reste modifiable, en plus pâle.
- **coupure** balaie un filtre **passe-bas**, **passe-haut** ou **passe-bande** entre les coupures **bas** et **haut** (le bas et le haut de la piste), avec de la **résonance**.
- **panoramique** déplace le son à gauche (en bas) et à droite (en haut) en baissant l’autre côté, jamais en le montant.
- **lissage** adoucit les arêtes (1 ms par défaut ; augmentez-le si une coupe claque). **mix** mélange le son sec.
- Les formes suivent la position du morceau : chaque cycle commence sur le temps, où que vous lanciez la lecture.

### correction de hauteur {#pitch-correction}

Accorde une voix seule ou un instrument soliste : il entend la note chantée ou jouée et la ramène sur la bonne.

- **tonalité** et **gamme** fixent les notes visées (chromatique par défaut : le demi-ton le plus proche). Avec **cible** sur **midi**, il vise la note tenue sur son entrée midi (choisissez une piste midi comme entrée midi de l’appareil) ; tant qu’aucune note n’est tenue, il revient à la gamme.
- **vitesse de correction** règle la rapidité : 0 ms colle à la note tout de suite (l’effet de voix « autotunée »), 40 ms corrige franchement mais naturellement, quelques centaines gardent le vibrato et les glissés. **humaniser** relâche la correction sur les notes longues pour qu’elles gardent leur mouvement, tandis que les changements de note restent rapides. **quantité** dose la correction, **transposition** déplace toute la voix en demi-tons.
- Avec **préserver les formants** (actif par défaut), la voix garde son timbre quel que soit le décalage. **formant** la rend plus grave ou plus claire, sans changer les notes. Désactivez **préserver les formants** pour l’effet « voix de souris » classique.
- **détecteur** choisit la méthode : **pyin** (par défaut) est plus stable sur les prises soufflées ou bruitées, **yin** est un peu plus léger. Montez la **sensibilité** si les notes douces ne sont pas corrigées, baissez-la si les respirations et les bruits sont accordés.
- Le graphe montre les 4 dernières secondes : la hauteur entendue (en pâle) et celle jouée, sur les notes de la gamme.
- Il retarde le son d’environ 34 ms, compensés à la lecture ; si vous vous écoutez en direct à travers lui, vous entendrez ce retard.

### eq adaptatif {#adaptive-eq}

Un eq qui écoute et rééquilibre le son en continu : il coupe ce qui dépasse (résonances, un bas qui bourdonne, un médium agressif) et fait ressortir ce qui est enfoui, vers une courbe tonale équilibrée. Idéal sur un bus de mix, le master ou un groupe chargé.

- **dompter** règle combien il coupe ce qui dépasse, **révéler** combien il fait ressortir ce qui est enfoui. Les deux à 0 laissent le son intact.
- **éclaircir** incline la courbe visée : vers le haut pour un résultat plus brillant, vers le bas pour plus sombre.
- **plage** limite le mouvement de chaque bande (6 dB par défaut). **vitesse** règle la rapidité avec laquelle il suit la musique : lent, c’est doux et transparent ; rapide, il suit chaque note (et peut pomper).
- **graves** et **aigus** fixent la plage de fréquences traitée (de 40 Hz à 16 kHz par défaut).
- **gain auto** garde la même sonie, pour entendre l’équilibre et non un son simplement plus fort. **geler** fige la courbe actuelle. **mix** mélange le son sec.
- Le graphe montre ce qu’il fait : la courbe appliquée au-dessus de barres qui situent chaque bande par rapport à la courbe équilibrée (vers le haut : elle dépasse, vers le bas : elle est enfouie).
- Il n’ajoute aucune latence.

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

### modeleur de volume {#volume-shaper}

Dessine le volume sur un cycle qui se répète en rythme avec le morceau : un pompage façon sidechain sans sidechain, des gates, des hachures et des montées. Il démarre sur le classique effacement par le kick à chaque temps.

- Le cycle dure une **vitesse synchro**, de 1/64 à 8 mesures avec les triolets (1/4 par défaut). Désactivez **synchro** pour régler une **vitesse** libre en hertz. Un cycle synchronisé suit la position dans le morceau, il tombe donc sur le temps où que démarre la lecture.
- Dessinez la forme : double-cliquez pour ajouter ou supprimer un point, faites glisser les points (ils s’alignent sur la grille, maintenez alt pour ne pas aligner), alt-glissez un segment pour le courber, clic droit pour un segment en escalier, linéaire ou lissé. Le haut est le volume plein, le bas le silence.
- **profondeur** dose toute la forme (0 % laisse le son intact), **phase** la décale dans le cycle, **phase stéréo** décale le côté droit, et **lissage** adoucit les arêtes pour que les marches ne claquent pas.
- **bandes** : la bande au-dessus de la forme montre les fréquences, de 20 Hz à gauche à 20 kHz à droite. Choisissez **1**, **2** ou **3** bandes : avec 2 ou 3, chaque bande (graves, médiums, aigus, chacune dans sa couleur) a sa propre forme, pour que les graves pompent pendant que les aigus restent en place. Faites glisser les traits entre les bandes pour déplacer les séparations (200 Hz et 15 kHz par défaut). Cliquez sur une bande, dans la bande des fréquences ou sur son onglet, pour dessiner sa forme ; sa **profondeur** et sa **vitesse** sont sous la forme (une vitesse de ×2 fait tourner le cycle de cette bande deux fois plus vite, ÷2 deux fois moins vite). Toutes les bandes au volume plein, le son passe intact, et la séparation n’ajoute aucune latence. Une bande pas encore dessinée reste au volume plein.
- Les préréglages couvrent les pompages de kick en sidechain (1/4 doux, classique et dur, 1/8, 1/2, demi-temps, effacement à contretemps), des gates trance, des hachures, des montées et du trémolo, et des préréglages par bande : **sidechain lows 1/4** (seuls les graves s’effacent), **pump lows, light mids** et **lows pump, highs chop**.

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
- **sonie** : un sonomètre (la mesure ebu r 128 qu’utilisent les plateformes de streaming et la diffusion). Il affiche la sonie **instantanée** (les 0,4 dernières secondes), **court terme** (les 3 dernières secondes) et **intégrée** (toute la lecture, en lufs, sans les passages silencieux), la **plage** (combien la sonie varie, en lu) et la **crête vraie** (le point le plus haut de la forme d’onde, entre les échantillons aussi, en dbtp ; en rouge au-dessus de -1). Le graphe montre l’historique ; la ligne pointillée est votre **cible** (-14 par défaut). Avec **réinitialiser à la lecture**, chaque lecture commence une nouvelle mesure. Placez-le en dernier sur le master pour vérifier un mix.

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
