# SaiLineArt — état de développement et reprise

Ce document résume l’état technique de SaiLineArt pour reprendre le travail sur une autre machine. Il complète le [guide utilisateur](sailineart.md) et renvoie aux rapports expérimentaux suivis par Git. Le journal historique `log/devlog.md` est ignoré par Git ; les décisions et constats utiles à la reprise sont donc consignés ici.

## État du dépôt

État vérifié le 20 octobre 2026 : branche `sailineart`, commit `23ca3d318991f01676d1f38f79a80eb00869e47e` (`feat(sailine): preserve linework development and latency diagnostics`), arbre de travail propre. La branche distante suivie `sailineart/sailineart` pointait sur le même commit. Le commit comprend l’optimisation de dégâts du cache et l’instrumentation de latence. Aucune migration vers PhotoCraft 0.10.0 n’a été effectuée.

## Fonctionnalités en place

SaiLineArt est intégré nativement à PhotoCraft comme type de calque éditable. Les tracés conservent leurs points de contrôle, identifiants stables, largeur de base, couleur, pression par point et état d’angle. Les caches de rendu sont dérivés ; la sauvegarde `.pcraft` conserve les données vectorielles. L’historique Engine prend en charge Undo/Redo.

Le moteur de courbe **SAI simplifié** est le modèle de production. Sa formule exacte est dans `crates/vector/src/linework.rs` et son étude reproductible dans `experiments/sailine_sai126/`. Le modèle optimisé demeure expérimental. La pression utilise le profil validé dans l’implémentation et n’a pas été modifiée par les travaux de performance.

Les sept outils livrés sont SaiCurve, SaiPressure, SaiLine, SaiWeight, SaiCombine, SaiPointed et SaiColor. Les raccourcis communs, la sélection multiple, la suppression/insertion de points et de segments, les transformations géométriques Ctrl+T, le mode pinceau de SaiCurve, le rendu rasterisable et la documentation utilisateur sont décrits dans [le guide utilisateur](sailineart.md). La déformation libre n’est pas présentée comme une parité SAI vérifiée.

## Architecture à connaître

- Modèle persistant : `crates/doc/src/linework.rs` et la variante `LayerContent::Linework`.
- Interpolation, profils de pression et contour : `crates/vector/src/linework.rs`.
- Commandes, cache par tracé, invalidation et historique : `crates/engine/src/linework_cmds.rs`.
- Interactions SaiLineArt : `crates/ui-egui/src/sailine_ui.rs`, avec intégration au canevas et aux outils de transformation.
- Instrumentation interactive : `crates/ui-egui/src/stroke_trace.rs`, `canvas.rs` et `gpu_canvas.rs`.
- Sérialisation native : manifeste et conversions dans `crates/format/` ; les caches restent transitoires.

SaiLineArt réutilise les surfaces, le canevas, les commandes Engine, l’historique et le rendu PhotoCraft. La principale couture d’intégration reste `LayerContent::Linework` et ses correspondances exhaustives dans les opérations document, le rendu, les exports et la sérialisation. Le détail de la cartographie est dans le [rapport de performance et migration](../experiments/sailine_performance/report.md).

## Dernier travail sur la latence

Le commit courant ajoute deux optimisations ciblées dans `crates/engine/src/linework_cmds.rs` :

1. Pour un calque à un seul tracé, la surface fraîchement rendue devient directement le cache de calque, sans composition redondante vers une seconde surface. Le cache compact par tracé est conservé pour les snapshots et l’historique. Les calques à plusieurs tracés continuent d’utiliser le compositeur existant afin de préserver l’ordre et le mélange alpha.
2. Pour le déplacement d’un ou plusieurs points dans un calque à un seul tracé, l’ancien et le nouveau raster sont comparés afin de fournir les bornes exactes des pixels modifiés. La zone comprend les pixels effacés à l’ancienne position. Cette comparaison réduit le travail en aval (rafraîchissement et transfert de texture) lorsque l’AABB du tracé est beaucoup plus grande que sa partie réellement modifiée. Les autres commandes et calques multi-tracés conservent des bornes conservatrices.

Le compteur `refreshed_pixels` est l’aire du rectangle endommagé, pas le nombre de pixels non transparents ou différents. Le compteur `gpu_uploads` provient de `Stats.tiles_uploaded` : il compte des tuiles de texture modifiées de 256 px et ne signifie pas nécessairement autant d’appels de transfert GPU. L’instrumentation CPU ne mesure pas le temps photon-à-photon ni la présentation physique à l’écran.

### Mesures disponibles et limites

Mesures graphiques **avant** le correctif, rapportées par l’utilisateur sur CachyOS KDE Wayland avec une Radeon RX 7600 Vulkan : 20–30 FPS ; 32–36 ms CPU/frame ; commande `linework.movePoints` 26–28 ms ; rafraîchissement SaiLineArt 22–24 ms ; environ 1 million de pixels dans les rectangles rafraîchis ; compteur de tuiles GPU autour de 28. Le pinceau natif rapportait environ 60 FPS et 2,03 ms CPU/frame. Aucune mesure graphique après correctif n’est encore consignée : ne pas annoncer de gain FPS ou de latence visuelle vérifié.

Le benchmark headless Release du cas diagonal 1 tracé × 3 points sur toile 2048², avec 3 échauffements et 20 répétitions, a donné `movePoints` p50 16,706 ms, p95 17,531 ms, max 17,697 ms ; la rasterisation du tracé p50 10,396 ms, p95 11,144 ms. Ce résultat n’est pas une mesure de latence d’interface et montre que ce cas reste coûteux côté CPU. Refaire les mesures sur une machine cible avant d’en tirer une conclusion générale.

L’interpolation, la pression et le rendu final n’ont pas été volontairement changés. Aucun nouvel aperçu temporaire ni moteur de rendu n’a été ajouté.

## Reprise recommandée

1. Confirmer le commit et l’état du dépôt (`git status --short --branch`, `git log -1 --oneline`).
2. Lancer l’application Release avec la trace activée :
   ```sh
   PHOTOCRAFT_STROKE_TRACE=/tmp/sailine-after.csv \
     CARGO_HOME=/mnt/SSD2/Codex/.tooling/cargo \
     cargo run --release -p photocraft --offline
   ```
3. Refaire le geste SaiLineArt à trois points et le geste pinceau natif, au même zoom et sur la même toile que la mesure de référence. Comparer les médianes et p95 de la commande, des étapes de rafraîchissement, de la composition, de l’upload, de l’aire endommagée, du compteur de tuiles et de l’âge de pointeur. Une frame soumise n’est pas une preuve qu’elle a déjà été présentée à l’écran.
4. Reproduire le benchmark CPU avec `bash experiments/sailine_performance/run.sh quick` ; pour la matrice étendue, `bash experiments/sailine_performance/run.sh full target/sailine-bench-full.json`. Le [README du banc](../experiments/sailine_performance/README.md) décrit les cas et les rapports archivés.
5. Si la latence reste élevée, cibler d’abord l’étape dominante observée. La comparaison exacte des pixels peut encore parcourir une grande AABB ; un tracé à trois points peut réellement changer sur une grande partie de sa courbe. Les déplacements dans les calques multi-tracés et les changements de pression n’ont pas encore cette réduction de dégâts exacte. Préserver l’effacement de l’ancienne position, la transparence, l’ordre alpha, les bords de toile et les snapshots Undo/Redo.

## Vérifications du commit courant

Avant le commit `23ca3d31`, les vérifications ciblées suivantes ont réussi :

- `cargo check -p photocraft --offline`.
- `cargo test -p photocraft-engine --offline --test linework` : 18 tests.
- Test unitaire `one_path_cache_matches_compact_composition_and_reports_exact_pixel_damage` : égalité des pixels, bornes modifiées, snapshots Undo/Redo.
- `cargo test -p photocraft-ui-egui --offline --lib stroke_trace::tests` : 4 tests.
- `git diff --check`.

Pour une modification future du chemin chaud, relancer les tests concernés et le benchmark, puis ajouter une comparaison pixel à pixel pour toute optimisation qui touche au cache ou aux bornes de dégâts.

## PhotoCraft 0.10.0

La migration est explicitement en attente. Le rapport de performance contient une évaluation préliminaire et un ordre de migration prudent, mais aucun diff source exact avec le tag 0.10.0 n’a été établi. Avant migration, récupérer et vérifier localement le tag officiel, travailler dans un checkout propre, puis comparer d’abord le modèle documentaire/calques, les commandes et l’historique, la sérialisation, les caches/rendu et enfin l’intégration UI. Ajouter des tests de round-trip `.pcraft`, Undo/Redo et rendu avant de porter les interactions. Ne pas fusionner directement cette branche dans une base 0.10.0 sans cet audit.

## Documents et expériences de référence

- [Guide utilisateur et référence des comportements](sailineart.md) et [version HTML autonome](sailineart.html).
- [Rétro-ingénierie de l’interpolation SAI](../experiments/sailine_sai126/README.md), données, scripts, diagnostic et planche de comparaison.
- [Rétro-ingénierie de la pression](../experiments/sailine_width/README.md), données, scripts, diagnostic et résultats.
- [Benchmarks, cache, architecture et compatibilité 0.10.0](../experiments/sailine_performance/README.md) et [rapport détaillé](../experiments/sailine_performance/report.md).
