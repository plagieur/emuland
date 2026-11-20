# emuland

Un **émulateur de Game Boy et de Game Boy Color** écrit en Rust, avec son propre programme de bureau.
Le processeur, l'écran, le son, le timer, les cartouches et le câble link sont écrits de zéro et vérifiés
avec les ROMs de test de référence de la communauté. Le cœur (`gb-core`) est indépendant de l'interface et s'utilise comme bibliothèque.

Voir les captures et les détails dans le [README en anglais](README.md). L'essentiel :

## Ce que ça fait

- **Game Boy et Game Boy Color** : couleurs, deux banques de VRAM, palettes, HDMA, vitesse double.
- Processeur exécuté **cycle machine par cycle machine** ; écran avec fond, fenêtre, sprites ; son à 4 voix ; timer ; bug du HALT.
- Cartouches : ROM seule, MBC1 (et multi-jeux), MBC2, MBC3 avec **horloge temps réel**, MBC5, MBC7 (accéléromètre piloté par la souris), HuC1, HuC3, Pocket Camera.
- **Câble link** entre deux consoles côte à côte.
- Bibliothèque de ROMs, sauvegardes, **4 états sauvegardés par jeu** (`F5` / `F8`), interface en français et en anglais (menu *Langue*), plein écran (`F11`), touches et souris réglables, joueur 2 en liaison.

## Précision

44 des 54 ROMs de test passent : tous les tests d'instructions et de timing de Blargg, et **dmg-acid2 / cgb-acid2 identiques à l'image de référence, pixel pour pixel**.
Manques connus : accès à la wave RAM pendant la lecture, bug de corruption de l'OAM, durée variable du dessin de chaque ligne ; cartouches MBC6, MMM01 et TAMA5 non gérées.

## Lancer

```bash
cargo run --release -p emuland
```

Importe tes propres ROMs (`.gb`, `.gbc`) avec le bouton *Importer une ROM* ou en les glissant sur la fenêtre.
**Ce dépôt ne contient aucun jeu ni aucune ROM de démarrage** : n'utilise que des copies de cartouches que tu possèdes.

| Game Boy | Clavier (par défaut) |
|---|---|
| Croix | `Z` `Q` `S` `D` ou les flèches |
| A / B | `L` / `K` (ou `X` / `C`, clic gauche / droit sur l'écran) |
| Start / Select | `Entrée` / `Retour arrière` |
| État : sauver / charger | `F5` / `F8` |
| Plein écran | `F11` (`Échap` pour sortir) |

Les sauvegardes, états et réglages sont dans `%APPDATA%\emuland`.

## Licence

[MIT](LICENSE). *Game Boy* et *Game Boy Color* sont des marques de Nintendo ; emuland n'est ni affilié à Nintendo ni approuvé par elle.
