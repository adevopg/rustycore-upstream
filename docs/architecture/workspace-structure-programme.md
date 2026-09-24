# Programa maestro de estructura del workspace

**Para qué sirve:** es el **índice y el orden de ejecución** de todo el trabajo estructural del
workspace. No duplica detalle: fija qué va antes de qué, qué evidencia cierra cada fase y en qué
estado está cada una, para que cualquier agente pueda retomar sin perderse.

**Autoridades (no compiten entre sí):**

| documento | gobierna |
|---|---|
| [structure-and-conventions.md](structure-and-conventions.md) | el **estándar**: capas, nombres, visibilidad, tests, presupuestos, checklist |
| [wow-world-distribution-plan.md](wow-world-distribution-plan.md) | el detalle de **wow-world** (forma objetivo y fases F0-F13) |
| este documento | el **orden, dependencias y estado** de todo el workspace |
| [refactor-completion-plan.md](refactor-completion-plan.md) | el plan técnico general del port (no lo sustituimos) |

**Regla de los dos planos:** la lógica se contrasta con la referencia C++ 3.4.3; la estructura la
decide este programa. Ninguna decisión estructural se justifica con "en C++ es así".

## 1. Estado de partida (auditoría medida, `3.4.3` + rama de distribución)

Reejecutable con el script de auditoría (mide líneas, ficheros, fichero mayor, deps internas,
consumidores, capa y violaciones) sobre `crates/`.

**Monolitos:** `wow-world` 410 725 (mayor 1 720) · `wow-data` 83 539 (1 999) · `wow-entities`
79 731 (**4 726**) · `world-server` 60 368 (**5 675**) · `wow-map` 54 927 (**5 133**) ·
`wow-packet` 50 339 (1 682) · `wow-database` 45 556 (1 793) · `wow-social` 5 354 (2 508).

**Muertos/stubs/mal declarados:** `wow-spell` (1 línea) · `wow-pvp` (1) · `wow-achievement` (1) ·
`wow-scripts` (44, depende de `wow-script`) · `world-modules` (23, **depende de `world-server`**) ·
`rustycore-db` (295, sin consumidor) · `wow-collections` (693, sin consumidor y **colisiona de
nombre** con el dominio de colecciones de cuenta) · `wow-session` (596, capa ambigua) ·
`capture-diff` (herramienta de QA dentro de las capas de juego) · `wow-recastdetour` (vendor sin
señalizar) · `wow-chat` (solapa con `wow-social`).

**Inversiones de capa:** `data→entities`, `data→movement`, `entities→loot`, `map→loot`,
`packet→loot`, `packet→movement`, `database→persistence`, `world-modules→world-server`, y las
laterales `ai→instances`, `conditions→loot`, `scripts→script`.

## 2. Secuencia maestra

Orden por **dependencias reales**: primero lo que quita ruido, después lo que quita tamaño, después
lo que cambia contratos, y al final la aceptación. Nada de una fase empieza si su predecesora no
está verde (ver §4).

### Ola A — saneamiento barato (no cambia contratos ni comportamiento)
| id | objetivo | evidencia de cierre | depende de |
|---|---|---|---|
| A1 | plegar `rustycore-db` en `wow-database`; retirar `wow-pvp`, `wow-achievement`, `wow-scripts` (o consolidar en `wow-script`); renombrar la utilidad `wow-collections`; decidir `wow-session` y `wow-chat` | workspace compila; recuento de crates baja; `inventory::submit!` y nº de tests invariantes | — |
| A2 | invertir `world-modules` para que no dependa de `world-server` | grafo sin esa arista | A1 |
| A3 | marcar tooling: `capture-diff` fuera de las capas de juego; renombrar el vendor de navmesh | grafo y capas coherentes con el estándar | A1 |

### Ola B — `wow-world` (detalle en su plan)
| id | objetivo | depende de |
|---|---|---|
| B1 | F5 ✅ `misc` desmontado en 13 dominios (commit `e719ac38`) | — |
| B2 | F6 dominios cohesivos a su crate (`reputation`, `planner`, `profession`/`trainer`, `entity_update_bridge`, `battle_pet_*`) | B1 |
| B3 | F10a tests por dominio: `loot_tests`, `quest_tests`, `character_tests`, `group_tests` a sus crates | B2 |
| B4 | F7 sub-estados dueños dentro de `WorldSession` (**diseño**) | B2 |
| B5 | F8 handlers grandes → adaptadores ≤600 líneas (**diseño**) | B4 |
| B6 | F9 `map_manager` + `map_manager_tests` → `wow-map` con contrato (**diseño**) | B4 |
| B7 | F10b resto de tests; F13 warnings y política de ownership regenerada con delta revisado | B5, B6 |

### Ola C — inversiones de capa (cada una es un movimiento pequeño)
| id | objetivo | depende de |
|---|---|---|
| C1 | tipos de registro de `wow-data` bajan a `wow-data`; se invierten `data→entities` y `data→movement` | A1 |
| C2 | los tipos que `entities`/`map`/`packet` necesitan de loot suben a su dueño; se rompen `entities→loot`, `map→loot`, `packet→loot` | C1 |
| C3 | `packet→movement` y `database→persistence` corregidas a su dirección natural | C2 |
| C4 | laterales de L3 (`ai→instances`, `conditions→loot`, `scripts→script`): fijar dirección por dominio y documentarla | C3 |

### Ola D — los siguientes monolitos (mismos presupuestos del estándar)
| id | objetivo | prioridad | depende de |
|---|---|---|---|
| D1 | **`wow-entities`**: fichero mayor 4 726 → ≤1 000; separar modelo canónico de estado de gameplay | alta | C2 |
| D2 | **`world-server`**: `app.rs` 5 675 → módulos por fase; composición delgada | alta | B7 |
| D3 | **`wow-map`**: ficheros 5 133 → ≤1 000; separar runtime de grillas de la fachada | media | B6 |
| D4 | **`wow-data`** y **`wow-database`**: techos de fichero y organización por catálogo/tabla | media | C1 |
| D5 | **`wow-packet`** y **`wow-social`**: techos de fichero; packet solo wire | media | C3 |

### Ola E — cierre
| id | objetivo | depende de |
|---|---|---|
| E1 | auditoría del estándar en todo el workspace (mismo script): 0 stubs, 0 sin consumidor, 0 inversiones, 0 fichero > 1 000 | A-D |
| E2 | ledger y política físicas consistentes con el árbol (delta revisado) | E1 |
| E3 | **validación final única** (`./tools/validation-v2 final --architecture`) y evidencia publicada | E2 |
| E4 | medición de build y latencia de herramientas antes/después (#1231); se publica tal cual, incluso si es negativa | E1 |

## 3. Registro de estado

Se actualiza **en el mismo commit** que cierra cada fase. Convención: `[ ]` pendiente, `[~]` en
curso, `[x]` cerrada con commit.

```
A1 [ ]  A2 [ ]  A3 [ ]
B1 [x] e719ac38   B2 [ ]  B3 [ ]  B4 [ ]  B5 [ ]  B6 [ ]  B7 [ ]
C1 [ ]  C2 [ ]  C3 [ ]  C4 [ ]
D1 [ ]  D2 [ ]  D3 [ ]  D4 [ ]  D5 [ ]
E1 [ ]  E2 [ ]  E3 [ ]  E4 [ ]
```

## 4. Qué significa "verde" en cada nivel (no confundir niveles)

1. **Compila**: `cargo check -p <crate>` (y sus consumidores). Es el bucle de trabajo, no evidencia
   de entrega.
2. **Suite**: los tests del crate afectado, y si se movieron tests, los invariantes contados
   (`#[test]`/`#[tokio::test]`, `inventory::submit!`, lista de escenarios).
3. **Composición**: `cargo check -p world-server --tests` y los targets de integración.
4. **Aceptación**: la campaña final única. Solo aquí se habla de aceptado.

Ninguna fase se declara cerrada con el nivel 1; la tabla de estado registra el nivel alcanzado.

## 5. Reglas para no perdernos

1. **Una fase, un commit**, con su mensaje diciendo qué cambió y qué se verificó.
2. **Movimiento y comportamiento separados**, siempre.
3. **Nunca** subir un techo, regenerar una política o retirar un test para que pase una fase.
4. Si una fase revela trabajo no previsto, **se añade a este documento** antes de hacerlo.
5. Al cerrar cada ola, se reejecuta la auditoría y se actualiza §1 y §3.
6. Nada se publica (push/PR/merge) sin autorización explícita.
