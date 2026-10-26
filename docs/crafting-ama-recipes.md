# Crafting reference recipes from Wuslwiz's AMAs

Reviewed 2026-10-26 against the 46 sheet recipes in [fixtures/guides](../fixtures/guides). These **19 supplemental references** replace the earlier mixed 60-entry research catalogue. They cover the [3.29][ama329], [3.28][ama328], [3.27][ama327] and [3.26][ama326] AMAs, retaining ordinary crafting methods that add a distinct target, mod pool or operation sequence beyond the sheets.

These are human recipe references for future tests, not new executable TOML fixtures or simulator-validated costs. Each source links to the answer and its request. Missing bases, tiers and item levels remain fixture choices rather than invented source facts. Historical prices, profit claims and estimated attempt counts are omitted.

## Conventions

- P/S mean prefix/suffix. `1P + 1P → 2P` combines isolated prefixes; `AC + BC → ABC` combines overlapping two-mod donors. Rebuild or recycle donors according to what survives. No crafted-filler probability boost is assumed.
- Starting fractures, influences and synthesised implicits are explicit inputs. A bought prerequisite is not a recipe for manufacturing that prerequisite.
- Locks must be reapplied after they are removed. Wild Bristle Matron guarantees Suffixes Cannot Be Changed only with full suffixes and an available prefix; otherwise use the bench lock.
- A two-mod rare and a two-mod magic item are different inputs. Check rarity and available slots before multimodding or applying locks.
- The answer supplies the main operations. Retry notes use conservative failure handling: when a wanted mod is lost or no safe recovery is supplied, restart the affected stage/base. They do not claim an optimal salvage policy, free annuls or an undo.
- A targeted Harvest **reforge** is different from an **augment/add-remove**. The latter can remove another wanted modifier. A claimed forced result applies only to the stated base, influence, blocked groups and occupied slots; it does not guarantee a good tier.

## Jewels and clusters

### 01. RF maximum-fire-resistance jewel

**Reference:** `ama-328-rf-max-fire-jewel` · [3.28 request and answer][rf-jewel]

- **Start and target:** Ordinary jewel with fractured +2% maximum fire resistance; add increased maximum life and fire damage over time multiplier.
- **Recipe:** Buy the fractured jewel, or obtain the maximum-resistance roll, Harvest-swap it to fire and fracture it. On the fractured base, repeat Harvest reforge fire until both remaining targets appear. An Exalt on a spare prefix is optional and outside the target.
- **Retry:** A wrong fracture needs another candidate. A failed reforge stays in the reforge loop; the wanted fracture survives.
- **Adds beyond the sheets:** Ordinary-jewel affix limits, a fractured maximum-resistance suffix, and a two-target tagged-reforge loop. The sheet set has no ordinary-jewel recipes.
- **Fixture choices:** Fix the life/multiplier thresholds. If fracture production is tested too, prepare a legal fracture candidate rather than fracturing a one-mod jewel.

### 02. Physical-minion Ghastly Eye Jewel

**Reference:** `ama-327-minion-physical-speed-jewel` · [3.27 request and answer][ghastly]

- **Start and target:** Ghastly Eye Jewel; flat physical damage added to minion attacks and minion attack speed.
- **Recipe A:** Repeat Corroded + Jagged + Shuddering in a three-socket resonator until both modifiers appear.
- **Recipe B:** If T1 physical damage is required, buy or make that fractured prefix, then Harvest reforge speed until minion attack speed appears.
- **Retry:** Repeat the chosen fossil/reforge operation. Wrong fractures need another base. Neither route guarantees arbitrary extra affixes or good tiers on every attempt.
- **Adds beyond the sheets:** Abyss-jewel mod pools and a meaningful fossil-versus-fracture comparison. Keep the physical version explicit; the answer does not supply a separate elemental-minion fossil recipe.

### 03. Doryani's Lesson cold cluster

**Reference:** `ama-327-cold-three-notable-cluster` · [3.27 request and answer][cold-cluster]

- **Start and target:** Large cold-damage cluster with fractured Doryani's Lesson; add Blanketed Snow and Prismatic Heart.
- **Recipe:** Obtain the fractured notable, then repeat Dense + Prismatic + Pristine in a three-socket resonator until all three notables are present.
- **Retry:** Failed fracture preparation needs another candidate. Once the fracture is correct, fossil failures simply repeat.
- **Adds beyond the sheets:** A concrete three-notable cluster target and a fossil combination that removes competing filler pools. No cluster fixtures exist in the sheets.
- **Fixture choices:** Pin passive count and item level. The pinned mod data puts Doryani's Lesson in a suffix and the other two in prefixes; both Doryani's Lesson and Blanketed Snow require item level 68. An 8-passive choice would be a fixture decision, not a detail stated by the answer.

### 04. Overlord staff cluster

**Reference:** `ama-326-overlord-staff-cluster` · [3.26 request and answer][overlord]

- **Start and target:** Mace/staff large cluster; Overlord and Fuel the Fight. Martial Prowess is the request's optional third notable.
- **Recipe:** Repeat Harvest reforge attack until the chosen notable combination appears. Use separate acceptance predicates for the two-notable and three-notable versions.
- **Retry:** Keep reforging any outcome that misses the selected predicate. Overlord alone is not success for the two-notable target.
- **Adds beyond the sheets:** Multiple notable requirements in one tagged-reforge outcome, including an optional stricter target without changing the recipe.
- **Fixture choices:** Pin passive count and base level. In the pinned data Overlord requires item level 75; Fuel the Fight is also a prefix and Martial Prowess a suffix. The author's estimate for hitting Overlord alone is not an estimate for either full target.

## Rings and belts

### 05. Fractured Despair, DoT multiplier and life ring

**Reference:** `ama-329-despair-dot-ring` · [3.29 request and answer][despair-ring]

- **Start and target:** Non-influenced Delve ring with fractured Despair on hit; essence DoT multiplier, an elemental resistance, life and a crafted mana-cost prefix.
- **Recipe:** Use the DoT-multiplier essence until a satisfactory resistance appears with an open prefix. With the curse, multiplier and resistance filling suffixes, apply Matron/lock and Harvest reforge life. Repeat the lock/reforge stage until acceptable life and a spare prefix; craft mana cost.
- **Retry:** Lost ordinary suffixes return to essence rolling. If life reforging fills prefixes and no safe recovery is available, restart from the fractured base; do not assume a free prefix annul.
- **Adds beyond the sheets:** A bought drop-only curse fracture plus an essence suffix. The Cogwork sheet instead fractures DoT multiplier and has a different slot layout and life-finishing method.
- **Input check:** Use the Delve affix, not a Hunter-influenced Despair ring. The pinned data distinguishes `CurseOnHitLevelDespairDelve` from `CurseOnHitDespairInfluence2`; the influenced version is not a substitute for this fracture.

### 06. Dual-Redeemer cold prefixes on a Cryonic Ring

**Reference:** `ama-329-redeemer-cold-ring-prefixes` · [3.29 request and answer][cryonic-ring]

- **Start and target:** Redeemer Cryonic Rings; added cold damage against chilled/frozen enemies and added cold damage per frenzy charge on one output.
- **Recipe:** Prepare one donor with each wanted prefix isolated. Recombine `1P + 1P → 2P`, rebuilding or recycling donors until both survive on an eligible output.
- **Retry:** A result retaining just one target can become a donor again. Losing both requires new donors.
- **Adds beyond the sheets:** Two influenced ring prefixes with different conditionals, rather than jewellery suffix work or elemental weapon rolls.
- **Scope:** The exact terminal goal is the two-prefix base. The source leaves suffix preferences unspecified, so no particular suffix finish is inferred. In the pinned data the frenzy prefix starts at item level 75; its highest tier needs 80.

### 07. Dexterity/resistance ring with unveiled life

**Reference:** `ama-327-dex-resist-veiled-life-ring` · [3.27 request and answer][dex-ring]

- **Start and target:** Ordinary ring, with Unset suggested in the request; dexterity, elemental resistance, chaos resistance and unveiled life.
- **Recipe:** Dexterity essence until the elemental resistance, with a free suffix and prefix. Bench Suffixes Cannot Be Changed, then reforge chaos. Once suffixes are complete, Matron/lock, Veiled Chaos, block mana if there is room and unveil life. Choose a compatible final prefix craft.
- **Retry:** Failed suffix preparation returns to essences. Failed veils can be cleared with a suffix lock and Scour when the lock fits; otherwise use a risky cleanup or restart. A full prefix set is not automatically repairable.
- **Adds beyond the sheets:** A jewellery prefix-unveil loop that cannot use Eldritch currency. The final bench choice and exact resistance tiers are fixture inputs, not supplied targets.

### 08. Intelligence/strength/cast-speed ring with ES

**Reference:** `ama-326-attributes-cast-speed-es-ring` · [3.26 request and answer][attribute-ring]

- **Start and target:** Ordinary three-suffix ring; intelligence, strength, cast speed, energy shield and crafted channelled-skill mana cost.
- **Recipe:** Intelligence essence until satisfactory strength. Lock suffixes and reforge caster for cast speed. Lock suffixes and Scour unwanted prefixes; reapply the lock and Harvest augment defence for ES. Repeat prefix clearing and the augment if the ES tier is too low. Craft channelled mana cost.
- **Retry:** Bad suffix tiers return to the essence stage. Prefix retries preserve the completed suffixes through the lock; include the add/remove operation and any consumed lock in the state.
- **Adds beyond the sheets:** Caster-tag suffix completion followed by defence-tag prefix augmentation on jewellery.
- **Scope:** Use the ordinary-ring version the author explicitly allows. The optional random fourth suffix on a Cogwork Ring is omitted, avoiding another Cogwork-sheet variant.

### 09. Strength/intelligence/Hunter-attributes belt

**Reference:** `ama-327-hunter-attribute-belt` · [3.27 answer][attribute-belt]

- **Start and target:** Non-influenced, non-fractured belt; intelligence, T1 strength, Hunter increased attributes, unveiled life and crafted elemental damage with attacks.
- **Recipe:** Intelligence essence until T1 strength, exactly two filled prefixes, one open prefix and one open suffix. Hunter Exalt, aiming for increased attributes. With the successful suffix set full, Matron/lock, Veiled Chaos, block mana when possible, unveil life, then craft elemental damage with attacks.
- **Retry:** A wrong Hunter suffix is a failed target; use another base rather than repeat the same pre-influence slam on it. Failed veils need a legal protected-prefix recovery or another attempt/base.
- **Adds beyond the sheets:** A belt target and an influence slam directed to a suffix by occupied slots. There are no belt fixtures in the sheet set.
- **Fixture choices:** The item image supplies no recoverable belt base or exact percentage-attributes tier. Pin those before testing; do not claim the influence slam guarantees the desired modifier.

### 10. Horror frenzy-cold ring with attributes

**Reference:** `ama-327-horror-frenzy-cold-ring` · [3.27 request and answer][horror-ring]

- **Start and target:** Purchased synthesised ring; Horror's cold damage per frenzy prefix, strength, intelligence, chaos resistance and unveiled life. Creating the implicit is outside this recipe.
- **Recipe:** Fundamental + Glyphic + Sanctified in a three-socket resonator until the wanted essence prefix and attributes appear. With a prefix and suffix available, lock suffixes and augment chaos. Reapply the suffix lock, Veiled Exalt, block mana where possible and unveil life. Select a compatible final craft.
- **Retry:** The augment or Veiled Exalt can remove the essence prefix. Losing it returns to fossils. A failed unveil requires a legal recovery or restart; the prefix is not protected merely because the suffixes are locked.
- **Adds beyond the sheets:** Glyphic's competing essence outcomes, attribute-biased fossil rolling, and two add/remove risks to a wanted prefix. This is different from the dual-influenced Redeemer prefix-pair recipe above.

## Weapons and quivers

### 11. Holy Relic trigger sword with +1 socketed gems

**Reference:** `ama-328-holy-relic-trigger-sword` · [3.28 request and answer][holy-relic]

- **Start and target:** Non-influenced thrusting sword; essence minion damage, natural attack speed, +1 socketed gems, crafted trigger and Hits Cannot Be Evaded.
- **Recipe:** Fear essence until acceptable attack speed. Annul or split down to a rare retaining just those two affixes. Craft Multimod plus Cannot Roll Attack Modifiers, filling all three suffix slots. Exalt for +1 socketed gems. Remove crafts, then Multimod trigger and Hits Cannot Be Evaded.
- **Retry:** Losing minion damage or attack speed during isolation returns to essences. A split result must actually retain both before continuing.
- **Adds beyond the sheets:** A forced non-attack prefix on a melee weapon, with a complete six-affix finishing arrangement. The minion snapshot wand sheet targets socketed strength/intelligence/support gems instead.
- **Slot check:** Before the Exalt: one natural prefix, one natural suffix and two crafted suffixes. The claim about +1 applies to the stated non-influenced thrusting-sword pool.

### 12. Crusader physical-DoT Citadel Bow

**Reference:** `ama-328-crusader-physical-dot-bow` · [3.28 request and answer][physical-bow]

- **Start and target:** High-level Crusader Citadel Bow; Contempt flat physical, generic attack DoT multiplier, Crusader physical/projectile-speed hybrid, crafted physical scaling and a chosen crafted suffix.
- **Recipe:** Contempt essence until DoT multiplier, then annul to exactly that prefix and suffix on a rare. Craft Multimod, Prefixes Cannot Be Changed and Suffixes Cannot Be Changed. Harvest reforge speed into the only open prefix. Remove crafts; Multimod increased physical damage and either attack speed or physical DoT multiplier.
- **Retry:** Failed isolation returns to essences. Reaching the protected slot arrangement is mandatory; do not apply this as an unrestricted reforge.
- **Adds beyond the sheets:** A physical DoT bow and simultaneous prefix/suffix protection. Existing bow sheets target elemental prefixes.
- **Data check:** The pinned `LocalIncreasedPhysicalDamagePercentAndProjSpeedInfluence*` modifiers remain Crusader bow prefixes; their tier requirements run from item level 68 to 80. Quality/Tempering is outside the affix test.

### 13. Hunter double-DoT quiver

**Reference:** `ama-328-hunter-double-dot-quiver` · [3.28 request and answer][hunter-quiver]

- **Start and target:** Hunter Vile Quiver; chaos DoT multiplier with attack skills and generic DoT multiplier with attack skills, plus crafted attack speed. Prefix quality can be a separate stricter target.
- **Recipe:** Alteration-roll each suffix as a solo mod on separate Hunter donors. Recombine until both survive on a two-mod rare. Craft Cannot Roll Attack Modifiers and Exalt prefixes. If unacceptable, lock suffixes/Scour, restore the attack blocker and repeat. Fill prefixes, remove the blocker and craft attack speed.
- **Retry:** Recycle useful recombination failures. Protected prefix cleanup must preserve both suffixes and leave room for the blocker again.
- **Adds beyond the sheets:** No quiver guides exist; this exercises suffix recombination followed by blocked Exalts on an influenced item.
- **Target distinction:** The request mentions physical DoT, but the answer deliberately rolls the generic attack-DoT suffix. Hunter prefixes remain possible; the Exalts do not guarantee only life and bow damage or high tiers.

### 14. Torment quiver with a final Hunter slam

**Reference:** `ama-326-torment-hunter-fire-dot-quiver` · [3.26 request and answer][torment-quiver]

- **Start and target:** Initially non-influenced quiver; Torment flat lightning, generic attack DoT multiplier, life, bow damage, crafted attack speed and Hunter fire DoT multiplier.
- **Recipe:** Torment essence until the generic multiplier; annul to those two mods. Craft Multimod plus Cannot Roll Attack Modifiers and Exalt twice for prefixes. Remove crafts, craft attack speed, then Hunter Exalt for the desired fire-DoT suffix.
- **Retry:** Losing either starting mod during isolation returns to essences. A wrong final influenced suffix fails the target and needs another base or a different project.
- **Adds beyond the sheets:** The non-influenced pool is used for prefix filling before influence is added. This is a different operation order and goal from the already-influenced double-DoT quiver.
- **Fixture choices:** Pin the starting quiver and tier thresholds, and confirm the non-attack prefix pool before treating both prefix types as forced. Neither their tiers nor the final Hunter result are guaranteed.

### 15. Budget mana-RF sceptre

**Reference:** `ama-328-budget-mana-rf-sceptre` · [3.28 request and answer][mana-rf]

- **Start and target:** Sceptre; natural mana prefix, natural generic DoT multiplier suffix, crafted fire DoT multiplier and crafted spell damage.
- **Recipe:** Roll solo mana on one magic donor and solo DoT multiplier on another. Recombine until both survive, then obtain the rare item state and room needed to Multimod fire DoT multiplier plus spell damage.
- **Retry:** Rebuild or recycle donors after failed combinations. A magic one-prefix/one-suffix result cannot be treated as already having rare-item crafting capacity.
- **Adds beyond the sheets:** A budget mixed-side recombination followed by crafted completion, with a different target from the +2 spell weapons. It is the author's explicit cheaper alternative to the requested four-natural-mod weapon.
- **Fixture choices:** The source does not specify sceptre base, mana/multiplier tiers or a separate rarity-conversion sequence. Keep the desired rare donor-product stage explicit when encoding that transition.

### 16. Fire-DoT sceptre with three completed suffixes

**Reference:** `ama-327-fire-dot-spell-crit-sceptre` · [3.27 answer][fire-sceptre]

- **Start and target:** Sceptre; generic DoT multiplier, fire DoT multiplier and essence spell-critical chance suffixes, unveiled increased fire damage and crafted increased DoT damage.
- **Recipe:** Shrieking spell-critical essence until generic DoT multiplier for one pair. Recombine a separate spell-critical donor with isolated high fire DoT multiplier for the second pair. Combine the overlapping pairs until all three suffixes survive. Matron/lock, Veiled Chaos, block physical damage before unveiling, seek increased fire damage and craft DoT damage.
- **Retry:** Failed suffix combinations return to the relevant donor pairs. Failed prefix unveils repeat the protected veil stage only when cleanup is legal; otherwise restart.
- **Adds beyond the sheets:** A three-suffix recombination ladder followed by a rare prefix unveil. The budget mana sceptre instead stops at one natural prefix and one natural suffix.
- **Fixture choices:** Base and exact suffix tiers are not recoverable from the request image. The answer identifies the affixes and method, but does not justify a particular T1 acceptance predicate.

### 17. Archmage spell-damage/mana wand

**Reference:** `ama-326-archmage-mana-wand` · [3.26 answer][archmage]

- **Start and target:** Wand with fractured spell-damage/mana hybrid; Woe spell damage, T3-or-better mana and cast speed, and unveiled double damage.
- **Recipe:** Woe essence until acceptable mana. Open a suffix if needed, risking a restart. Lock prefixes and reforge caster until acceptable cast speed. Relock prefixes and Veiled Exalt, aiming to retain cast speed. Block damage-per-charge before unveiling, then seek double damage.
- **Retry:** Losing an ordinary prefix returns to essences. Losing cast speed or missing the unveil returns to protected suffix cleanup and the caster stage; a legal lock is required.
- **Adds beyond the sheets:** A fractured hybrid prefix, three completed caster prefixes and a suffix-add/remove risk. Existing caster wands focus on +2 gem levels.
- **Scope:** The source recommends T3-or-better mana/cast speed as the practical target. Stop the reference predicate at unveiled double damage. Its final “craft double damage” instruction does not name the variant, so do not invent a second compatible double-damage requirement.

## Shields

For the next two recipes, the common protected setup starts with **one natural prefix and one natural suffix on a rare**. Multimod is a suffix, Prefixes Cannot Be Changed is a suffix, and Suffixes Cannot Be Changed is a prefix. Together they give **2P/3S**, with exactly one open prefix. The targeted reforge must preserve both protected sides while adding that prefix; removing crafts then leaves the three wanted natural affixes.

### 18. +1 chaos, block and ES recovery on block

**Reference:** `ama-328-chaos-block-es-recovery-shield` · [3.28 request and answer][chaos-shield]

- **Start and target:** Shaper intelligence shield; T1 increased block prefix, ES recovery on block suffix and +1 chaos spell gems prefix.
- **Recipe:** Alteration-roll recovery and block on separate eligible Shaper donors. Recombine into the two-mod rare. Apply the protected 2P/3S arrangement above, then Harvest reforge chaos. Remove crafts and choose compatible Multimod finishing affixes.
- **Retry:** Rebuild or recycle failed donor combinations. Incorrect slot counts must be repaired before the reforge; a different open slot arrangement is a different recipe.
- **Adds beyond the sheets:** The ES-block sheet obtains three defence/block prefixes before recovery. This recipe starts with one prefix/one suffix and uses both locks to add a gem prefix by tag.
- **Fixture choices:** Pin the shield base and recovery tier. Only the core three natural modifiers are required; the answer leaves its finishing crafts flexible.

### 19. +1 minions, minion damage and life recovery on block

**Reference:** `ama-328-minion-life-recovery-shield` · [3.28 request and answer][minion-shield]

- **Start and target:** Shaper minion-capable spirit shield; natural minion-damage prefix, life recovery on block suffix and +1 all minion gems prefix.
- **Recipe:** Roll/isolate the recovery suffix and minion-damage prefix on separate donors. Recombine into the two-mod rare. Apply the protected 2P/3S arrangement, then Harvest reforge minion. Remove crafts and select compatible finishing crafts.
- **Retry:** Failed donor isolation/recombination returns to donor preparation. Do not substitute a fractured influenced base or an unrestricted minion reforge.
- **Adds beyond the sheets:** An influenced minion offhand and minion-tag completion. It shares the lock arrangement of 18 but tests a different tag, native mod family and eligible base set.
- **Base check:** The pinned Fossilised and Bone Spirit Shields have `focus_can_roll_minion_modifiers`; Titanium Spirit Shield does not. The native damage and gem prefixes require the former tag. Choose an eligible base before compiling the goal.

## Deduplication and exclusions

The comparison was against the existing 46 TOML sheet recipes, not just item names. A changed resistance, defence type, gem element or destination base was not enough to add another entry unless it changed a meaningful goal or method.

- **Sheet overlap removed:** High-ES and high-armour variants; elemental bows/daggers/wands; the usual +2 spell/minion weapons; closely related suppression/minion helmets, strike-range gloves and elevated spectre boots. Examples already covered are [ES/block shield](../fixtures/guides/es-block-shield.toml), [+2 minion wand](../fixtures/guides/minion-plus-two-wand.toml), [physical DoT wand](../fixtures/guides/plus-two-physdot-wand.toml), [strike-range gloves](../fixtures/guides/strike-range-gloves.toml) and [spectre boots](../fixtures/guides/tailwind-plus-two-spectre-boots.toml).
- **League-dependent routes removed:** Allflame/Foulborn previews, sulphur/choice loops, Genesis Tree base production, Infamous or memory-strand crafts, Breach-mod transfers and special talisman rare-imprint routes. No such operation is part of the 19 recipes.
- **Insufficient answers removed:** Image-only targets, vague “recombine and pray” advice, unanswered requests, unexpanded redirects to a spreadsheet, and disputed or corrected guarantees without a clear final recipe. The three-T1-minion-stat Bone Ring request, for example, has no concrete author-supplied method worth promoting here.
- **No silent upgrades:** The Cryonic reference ends at two prefixes; the Hunter quiver uses the suffix the answer actually supplies; the Archmage reference leaves the ambiguous last bench variant unconstrained. None claims a complete original six-affix screenshot target from incomplete text.

## Turning references into tests

The first useful additions are 01–04 for entirely new jewel/cluster pools, 11 for a narrowly blocked sword Exalt, and 12/18/19 for simultaneous affix locks during targeted reforges. The belt and two quiver routes add influence-slam cases absent from the sheet goals.

Keep source predicates separate from implementation status. The pinned raw data includes ordinary-jewel mods in the `misc` domain, abyss-jewel mods in `abyss_jewel`, and cluster notables in `affliction_jewel`; the current rules loader does not yet import those domains. Those recipes remain valuable references, but cannot be called passing fixtures merely because the source mods exist in the compressed data.

For an executable fixture, choose the unresolved base/level/tiers, spell out purchased starting states, compile the exact mod templates and origins against the pinned snapshot, encode failure transitions, and compare recipe enumeration with real-item sampling. A partial request becomes a test only for its explicitly stated terminal subgoal. No solver, prices or existing sheet fixtures were changed for this curation.

[ama329]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/ama_crafting_ama_for_329_curse_of_the_allflame/
[ama328]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/
[ama327]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/ama_crafting_ama_for_327_keepers_of_the_flame/
[ama326]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/ama_crafting_ama_for_326_secrets_of_the_atlas/
[rf-jewel]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae1vel/
[ghastly]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnrhvlx/
[cold-cluster]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnyvk5l/
[overlord]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzpiul/
[despair-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11eqmy/
[cryonic-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yz4er/
[dex-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsez4n/
[attribute-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz1o6ha/
[attribute-belt]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxsv6n/
[horror-ring]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxr9qu/
[holy-relic]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oajpqaz/
[physical-bow]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaf49c4/
[hunter-quiver]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaeuckp/
[torment-quiver]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzd2p1/
[mana-rf]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oaeq46w/
[fire-sceptre]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsbrm6/
[archmage]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz4g4r2/
[chaos-shield]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae6mtb/
[minion-shield]: https://www.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/comment/oae5981/
