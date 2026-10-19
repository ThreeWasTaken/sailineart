# SaiLineArt / SAI 1.2.6 Beta 3 : banc de rétro-ingénierie

Ce dossier est une expérience autonome. Le calcul de production dans
`../../crates/vector/src/linework.rs` n'est pas modifié. L'image fournie par
l'utilisateur est conservée dans `reference.png` ; `dataset.json`,
`diagnostic.png`, `results.json` et `comparison_board.png` sont des résultats
reproductibles, pas des courbes saisies à la main.

## Reproduire

Depuis la racine de PhotoCraft :

```sh
python3 experiments/sailine_sai126/extract.py
cargo run --release --manifest-path experiments/sailine_sai126/Cargo.toml
magick -background white experiments/sailine_sai126/comparison_board.svg experiments/sailine_sai126/comparison_board.png
```

ImageMagick (`magick`) est la seule dépendance de l'extracteur Python, qui utilise
par ailleurs la bibliothèque standard. Le programme Rust a son propre workspace
et son propre `Cargo.lock`. Sur la machine de recherche, les dépendances Rust
étaient en cache dans `../.tooling/cargo` et le répertoire de compilation était
celui de PhotoCraft :

```sh
CARGO_HOME=/mnt/SSD2/Codex/.tooling/cargo \
CARGO_TARGET_DIR=/mnt/SSD2/Codex/photocraft/target \
cargo run --offline --release --manifest-path experiments/sailine_sai126/Cargo.toml
```

## Extraction vérifiée

L'image de 1306 × 874 pixels contient 51 marqueurs de contrôle de cinq pixels :
24 jaunes et 27 verts. Les composantes rouges sont affectées aux douze formes
numérotées ; les marqueurs sont ordonnés après inspection de leurs connexions.
Le programme ne déduit aucun contrôle d'un croisement ou d'un extremum. Il
remplace les trous laissés par les marqueurs dans le masque rouge, amincit le
trait par l'algorithme de Zhang–Suen et relie chaque paire consécutive par un
trajet sur le squelette. L'ordre et les trajets sont visibles dans
`diagnostic.png` : les douze formes y correspondent à l'original, y compris les
segments très rapprochés des cas 6, 7 et 9.

Le quadrillage détecté a une période de 38,378 px en x et 38,232 px en y ; les
origines ajustées sont 11,589 et 12,358 px. Son ajustement a une erreur
quadratique d'environ 0,6 px. Les centres des marqueurs sont conservés avec
une incertitude de ±1 px, le squelette avec ±1,5 px. Les coordonnées en pixels
et en cases de quadrillage sont présentes dans `dataset.json`, avec le SHA-256
de l'image source. Aucun trajet n'est signalé comme déconnecté. L'orientation
des points transcrits n'est pas une observation du sens des clics dans SAI.

## Algorithme PhotoCraft conservé comme témoin

La tangente de départ d'un point intérieur est `(P[i+1] − P[i−1])/2`, comme
dans le Catmull–Rom uniforme. Son amplitude dépend ensuite du rapport entre
les longueurs voisines, via plusieurs constantes et fondus (`1,15`, `0,15`,
`1,4`, bornes `0,4` et `0,6`). Les poignées entrante et sortante sont ajustées
séparément. Lorsqu'un retour est plus court que l'arrivée, la direction
sortante est en outre mélangée avec celle du segment suivant : un point peut
donc perdre la continuité géométrique de sa tangente. Les extrémités utilisent
une correction distincte, notamment des fondus selon le rapport des longueurs
et des angles de 110–140°. Deux points avec poignée active suivent un cas
particulier quadratique ; sans poignée, ils donnent une droite. Les points
superposés rétractent les poignées pour produire un angle dur.

Le banc appelle directement `photocraft_vector::linework::interpolate_points`
pour ce témoin : sa géométrie n'est pas recopiée. Comme elle dépend du sens,
`results.json` rapporte le score dans l'ordre transcrit et le meilleur des
deux ordres pour chaque cas. Ce dernier est le témoin équitable utilisé dans
les comparaisons ci-dessous.

## Modèles et mesure

Pour chaque famille Catmull–Rom, `h[i] = |P[i+1] − P[i]|^α`. Les valeurs
`α = 0`, `1/2` et `1` désignent respectivement les paramétrisations uniforme,
centripète et chordale. La dérivée intérieure est la moyenne pondérée des
deux sécantes paramétriques. Une tension globale `τ` multiplie chaque dérivée
par `1 − τ`. Quatre règles d'extrémité et neuf gains intermédiaires ont été
explorés. Les deux points seuls restent une droite.

Les références suggéraient que la paramétrisation seule ne suffisait pas,
notamment au cas 8. La famille locale ajoutée multiplie **la même dérivée
des deux côtés d'un point** par

`(min(longueur précédente, longueur suivante) / max(...))^β`.

Ce facteur est symétrique, continu hors dégénérescence et conserve une
tangente unique au point. Il ajoute un paramètre global `β`, sans règle liée
à un numéro de cas. Les points exactement superposés rétractent leurs
poignées ; ils constituent l'exception intentionnelle à la tangente lisse.
Cette extension n'hérite pas automatiquement de la garantie théorique de la
paramétrisation centripète contre les boucles *à l'intérieur d'un segment*.
Voir [Yuksel, Schaefer et Keyser](https://www.cemyuksel.com/research/catmullrom_param/catmullrom_cad.pdf).

Les courbes sont comparées **segment par segment** dans le même ordre de
contrôles. Pour chaque paire, le banc échantillonne la cubique et le squelette
de référence, et mesure les distances point–polyligne dans les deux sens.
Il rapporte moyenne, 95e percentile et maximum. Le score d'un cas vaut
`moyenne + 0,25 × P95` ; le score global est la moyenne non pondérée des
douze cas. Une recherche à 16 échantillons par segment explore les paramètres ;
les candidats retenus sont remesurés à 60 échantillons. Aucune couleur de
pixel n'entre dans le score. Un diagnostic séparé mesure la distance de
dépassement au-delà de chaque corde, la distance parcourue à rebours sur
elle et l'écart de direction sur les huit premiers pixels aux deux extrémités.

## Résultats

39 087 combinaisons ont été évaluées. Le meilleur réglage fin est
`α = 0,3375`, `τ = −0,2`, `β = 0,4875`, tangentes nulles aux extrémités.
Une formule plus simple, `α = 1/3`, `τ = −0,2`, `β = 1/2`, obtient 0,470 px
contre 0,455 px : l'écart de 0,015 px est très inférieur à la précision de
l'extraction. C'est donc la formule recommandée pour une éventuelle
intégration après validation dans l'application.

| Modèle | Score global (px) |
|---|---:|
| Formule locale ajustée | 0,455 |
| Formule locale simple (`1/3`, `−0,2`, `1/2`) | 0,470 |
| PhotoCraft actuel, meilleur sens par forme | 1,845 |
| PhotoCraft actuel, sens transcrit | 2,245 |
| Catmull–Rom centripète standard, meilleur réglage | 2,664 |
| Catmull–Rom uniforme standard, meilleur réglage | 4,431 |
| Catmull–Rom chordal standard, meilleur réglage | 6,862 |

Les trois dernières familles ont été optimisées sur la même grille de tensions
et de conditions aux extrémités. Dans les trois, la meilleure condition
d'extrémité était la tangente nulle.

| Cas | Nouveau : moyenne / P95 / max (px) | Actuel, meilleur sens : moyenne / P95 / max (px) |
|---:|---:|---:|
| 1 | 0,257 / 0,515 / 0,561 | 0,248 / 0,500 / 0,561 |
| 2 | 0,258 / 0,600 / 0,692 | 0,585 / 1,317 / 1,561 |
| 3 | 0,284 / 0,630 / 1,030 | 0,634 / 1,658 / 2,242 |
| 4 | 0,281 / 0,643 / 0,920 | 0,887 / 1,927 / 2,416 |
| 5 | 0,289 / 0,732 / 1,107 | 1,116 / 2,696 / 3,219 |
| 6 | 0,273 / 0,644 / 0,989 | 1,515 / 3,625 / 4,491 |
| 7 | 0,260 / 0,634 / 0,995 | 1,312 / 4,548 / 5,453 |
| 8 | 0,407 / 0,975 / 1,428 | 2,990 / 5,153 / 5,704 |
| 9 | 0,247 / 0,601 / 1,000 | 0,844 / 2,477 / 3,189 |
| 10 | 0,306 / 0,722 / 1,546 | 0,717 / 1,807 / 2,436 |
| 11 | 0,322 / 0,764 / 1,351 | 1,747 / 3,831 / 4,597 |
| 12 | 0,253 / 0,630 / 1,016 | 1,227 / 3,753 / 4,466 |

Dans `comparison_board.png`, SAI est rouge, le meilleur modèle bleu,
PhotoCraft actuel orange. Les plus grands résidus du modèle local sont au
bas du cas 8 (max 1,43 px), à l'extrémité du cas 10 (1,55 px) et sur le grand
tour du cas 11 (1,35 px). Ces écarts sont voisins de l'incertitude du trait ;
ils ne justifient pas une nouvelle règle propre à ces formes. Les cas 6 et 7
vérifient visuellement les retours serrés et les courbes imbriquées. La
projection sur la corde de chaque segment n'a aucun recul supérieur à 0,5 px
dans SAI ni dans le modèle retenu ; aucun dépassement longitudinal supérieur
à 0,001 px n'est mesuré. Le témoin actuel dépasse de 0,06 et 0,14 px aux
cas 5 et 10. L'écart angulaire maximal aux extrémités est de 4,5° pour le
modèle retenu, contre 12,9° pour le témoin dans son meilleur sens. Les mesures
par cas figurent dans `results.json` sous `geometry_diagnostics`.

En validation à trois groupes, les paramètres retenus restent proches :
`α = 0,3375`, `τ = −0,2` à `−0,2125`, `β = 0,475` à `0,5125` ; les scores des
groupes tenus à l'écart sont 0,432, 0,443 et 0,570 px. En laissant chaque
forme à l'écart à son tour, `α` et `τ` restent inchangés et `β` est toujours
0,475 ou 0,4875. Une perturbation indépendante de ±0,5 px sur chacun des
51 contrôles, répétée 100 fois, donne 0,556 px au modèle choisi contre
1,881 px au témoin dans son meilleur sens ; le modèle choisi gagne 100 fois.

Sur 1 000 chaînes synthétiques aléatoires, l'erreur d'inversion d'ordre est
inférieure à `2×10⁻¹³` px, les segments hors des quatre voisins sont
inchangés, les courbes passent exactement par les contrôles et l'écart
angulaire maximal entre tangentes adjacentes est inférieur à `6×10⁻¹²` degré.
Le témoin actuel reste local et interpolant, mais l'inversion peut changer
la géométrie de 7,19 px dans ce test. Un balayage de 4 608 configurations
supplémentaires couvre des longueurs de `10⁻⁷` à `10³` px et des angles par
pas de 5° : aucune boucle interne n'a été détectée parmi les segments
échantillonnés du nouveau modèle. Un déplacement de 0,001 px y modifie la
géométrie d'au plus 0,00153 px hors quasi-dégénérescence, et de 0,00265 px
dans les configurations quasi dégénérées de cette grille.

## Portée de la conclusion

Cette concordance forte **ne démontre pas** que SAI emploie exactement ces
exposants ou des cubiques Catmull–Rom. La référence est une seule capture,
avec une épaisseur et une anticrénelage qui limitent la localisation de la
ligne médiane. Les formes de validation viennent du même dessin, bien
qu'elles aient des géométries différentes. La recherche a exploré plusieurs
familles cohérentes, mais pas toutes les splines possibles. La formule simple
est prête dans `src/main.rs` pour une intégration future ; il faudra encore
tester le ressenti pendant le tracé et l'édition dans PhotoCraft avant de
remplacer `linework.rs`.
