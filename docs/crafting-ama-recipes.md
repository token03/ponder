# Crafting recipes from Wuslwiz's AMAs

Extracted 2026-10-26 against overhaul at `d14992c`. This is a research catalog of 60 distinct crafting goals or methods, distilled into starting conditions, targets, operations and retry points. It is not a set of simulator-validated acceptance fixtures.

Sources inspected: [3.29 Curse of the Allflame][ama329], [3.28 Mirage][ama328], [3.27 Keepers of the Flame][ama327] and [3.26 Secrets of the Atlas][ama326]. These are the readable thread snapshots and selected comment pages, not a complete Reddit API export. Collapsed replies, deleted requests, item screenshots and external PoBs were not fully recovered. Older threads were followed where the recent AMA redirected readers; 3.18–3.25 were not crawled.

Each entry pairs the core item request with the supplied crafting answer. Wuslwiz is the answer author unless another contributor is named. Exact bases, item levels and tiers are retained when stated in text; missing values remain unspecified. Similarity to an existing guide is an editorial comparison, not proof of identical goals or methods. Historical prices, profit claims and estimated probabilities are not current quotes or validated solver inputs.

## Reading the methods

- P/S mean prefix/suffix; `2P + 2P → 3P` means recombining two items with overlapping pairs of desired prefixes, retrying until all three survive.
- Recombination means the unpredictable/PoE 1 method unless specified otherwise. Failed items can be reused when they retain useful modifiers.
- A prefix/suffix lock must be reapplied after an operation consumes or removes it. A lock protects existing affixes; it does not prevent filling empty slots.
- Wild Bristle Matron is used to guarantee Suffixes Cannot Be Changed only when suffixes are full and a prefix is available. With an open suffix, bench craft the lock unless a different setup has been checked.
- Eldritch prefix work requires eligible, non-influenced armour and Exarch dominance. Do not apply it to influenced items, jewellery, weapons or shields.
- Allflame choices are finite and consume sulphur and intangibility capacity. None of the methods below assumes unlimited previews or an ordinary imprint rollback through the Allflame bench. The author later withdrew the opening post's cheap-Foulborn economics after sulphur prices rose. [Cost correction][329-233].
- The 3.29 discussion disputes whether crafted modifiers improve recombination odds. This catalog does not include crafted filler as a probability boost. [Discussion][329-161].
- Guaranteed outcomes below describe the source's intended mod-pool setup. Check the current base, influence, item level, occupied slots and tags before converting a recipe to a runnable guide. In particular, the AMA warns about older weapon affix changes.

## 3.29: Allflame methods and other additions

### 01. High ES body armour from Woe essences

- **Start/goal:** High-level ES chest; essence flat ES, T1/T2 increased ES, hybrid ES, useful intelligence/resistance suffixes.
- **Method:** Deafening Woe until T1/T2 increased ES. Annul or beast craft unwanted affixes to open the third prefix. Allflame Foulborn Exalt looking for hybrid ES or a useful suffix, then Allflame ordinary Exalts to finish. Six-socket with Omen of the Jeweller and link with Black Morrigan if worth finishing.
- **Retry/scope:** Cleanup and slams can fail; restart or settle according to surviving affixes. Foulborn is a price-sensitive option. For an additional endurance-charge essence suffix, fracture it first, then use this method. Different method for the goal family in [cheap-high-es-regalia.toml](../fixtures/guides/cheap-high-es-regalia.toml).
- **Source:** [Opening post][ama329]; [endurance-charge variant][329-253].

### 02. Temple chilled-damage ES gloves

- **Start/goal:** ES gloves with the Puhuarte resistance/damage-against-chilled suffix and high ES prefixes.
- **Method:** Harvest swap the temple modifier to cold resistance/damage against chilled enemies. Make a rare with at least four affixes and Allflame fracture the temple suffix. Woe essence until T1/T2 increased ES, remove unwanted mods, then Allflame Foulborn Exalt for hybrid ES. Finish suffixes with ordinary Allflame Exalts or a bench craft.
- **Retry/scope:** Failed temple fractures have a salvage route in 15. Starting rarity changes how many Regals/Exalts are needed to reach four affixes. Related goal in [chill-convert-gloves.toml](../fixtures/guides/chill-convert-gloves.toml); this method targets ES and does not supply conversion.
- **Source:** [Opening post][ama329].

### 03. Three-notable elemental cluster jewels

- **Start/goal:** Large fire/cold cluster of the desired passive count and suitable item level; three desired notables. Exact notable sets are unspecified.
- **Method:** Alteration to one desired notable as the only mod. Allflame Foulborn Augment for a second notable; restart if none is offered. Allflame Foulborn Regal for another notable or useful affix, then ordinary Allflame Exalt for the final opportunity.
- **Retry/scope:** A recipe family, not an exact three-notable target. The author also suggests it for medium clusters and ordinary/abyss jewels. Sulphur cost invalidated the opening post's early-league economics; compare against ordinary currency and fossils.
- **Source:** [Opening post][ama329]; [cost correction][329-233].

### 04. Fractured abyssal-socket armour bases

- **Start/goal:** Eligible armour base; fractured abyssal socket suffix for subsequent crafting.
- **Method:** Put Hollowed + Fractured fossils into a two-socket resonator and use it through the Allflame bench. Select an outcome fracturing the socket if available.
- **Retry/scope:** The source describes the socket being generated before the fracture attempt. Failed outcomes require another attempt/base as appropriate; four previews do not guarantee success. Base production only, with no finished item or current profit estimate.
- **Source:** [Opening post][ama329].

### 05. Triple-element attack wand with critical suffixes

- **Start/goal:** Attack wand with three flat elemental prefixes, critical chance/multiplier and attack speed; this is not flat damage to spells.
- **Method:** Fracture one flat element, essence another until the third reaches T1/T2. Alternative: overlapping two-prefix recombination into three prefixes. Lock prefixes and Harvest reforge critical until good attack critical chance or multiplier, with two free suffix slots. Lock again, Allflame Veiled Exalt, block damage-per-charge before unveiling, unveil critical/speed and bench craft the missing suffix.
- **Retry/scope:** Recover bad veils by protected annul/clearing suffixes where possible; otherwise rebuild. Reforges can fill suffixes. Related family in [ele-dmg-wand.toml](../fixtures/guides/ele-dmg-wand.toml), whose explicit targets are spell damage rolls.
- **Source:** [Answer][329-31].

### 06. Recoup, light-radius and chaos-resistance ring

- **Start/goal:** Fractured recoup ring; accuracy/light-radius suffix, chaos resistance, flat life and crafted regeneration/mana.
- **Method:** Harvest reforge attack for accuracy/light radius with an open suffix and prefix. Bench lock suffixes, reforge chaos. With suffixes filled, Matron/lock and reforge life until acceptable life. Craft regeneration/mana and optionally Allflame Exalt a remaining prefix.
- **Retry/scope:** Bad chaos tier means restart or rescue with annul. Reapply the lock for life attempts; full prefixes require cleanup. Omit the answer's suggested Conqueror Exalt: a reply correctly identifies that the fractured starting base prevents it.
- **Source:** [Answer][329-46]; [fracture/influence correction][329-49].

### 07. Strength and DoT multiplier +2 chaos amulet

- **Start/goal:** Amulet with fractured +1 all skills; strength, T1 generic DoT multiplier, +1 chaos skills and an available finishing prefix.
- **Method:** Strength essence until T1 DoT multiplier with an open suffix/prefix. Lock suffixes and reforge caster for cast speed or chaos for chaos resistance as the third suffix. Open two prefixes, lock suffixes again, Harvest augment chaos for +1 chaos gems, then bench craft the final prefix.
- **Retry/scope:** Annul preparation can remove wanted suffixes; restart the essence stage if that happens. Keep the augment step distinct from reforge: a reforge can fill other prefixes and lose the intended spare slot. No exact amulet base was given.
- **Source:** [Answer][329-85].

### 08. Triple fire-resistance mercenary chest

- **Start/goal:** Chest with natural elemental resistance, unveiled elemental/chaos resistance and crafted elemental hybrid resistance, all contributing fire resistance; life/evasion prefixes.
- **Method:** Veiled Chaos until high elemental resistance plus a veiled suffix and open suffix. Bench an attribute before unveiling a hybrid elemental/chaos resistance. Harvest swap the applicable elemental resistances to fire, then craft a compatible fire/hybrid resistance. Finish eligible prefixes with Eldritch currency and Allflame Exalts.
- **Retry/scope:** Restart failed unveiled suffix setups. Confirm the intended resistance crafts coexist. The answer also offers a cheaper Rog life/evasion/resistance chest with crafted fire/chaos and Allflame slams, without the triple-resistance goal.
- **Source:** [Answer][329-96].

### 09. AG Bone Ring with minion damage, movement and attack/cast speed

- **Start/goal:** Bone Ring with fractured minion damage; minion movement speed, minion attack/cast speed, chaos resistance and life.
- **Method:** Fear essence for minion movement speed until natural minion attack/cast speed and a free suffix. Bench lock suffixes, reforge chaos. Block mana, then Allflame Exalt for life.
- **Retry/scope:** Bad chaos tier or lost speed requires redoing suffixes. This is Worst_Yorick_Eu's method explicitly endorsed by Wuslwiz. It supersedes the same thread's life-fracture recommendation. The older life-fracture fossil/Harvest method remains historical, not the preferred version here.
- **Source:** [Contributing answer][329-102]; [author endorsement][329-105]; [older alternative][328-249].

### 10. Breach-critical mercenary chest with abyssal socket

- **Start/goal:** Body armour with fractured Breach critical scaling from overcapped lightning resistance, abyssal socket and the desired corrupted-essence suffix.
- **Method:** Transfer/fracture the Breach mod onto the desired base. Fundamental + Glyphic + Hollowed + Jagged in a four-socket resonator until the desired essence outcome. Preserve completed suffixes and use Exarch-dominant Eldritch prefix work plus Allflame Exalts; craft the final mod.
- **Retry/scope:** Repeat fossils for the desired essence. The source's claimed 50:50 is not recalculated here. The exact essence modifier and finished prefixes need to be specified before making an acceptance case.
- **Source:** [Answer][329-111].

### 11. Twelve-passive minion cluster

- **Start/goal:** Twelve-passive minion large cluster; 35% effect, 3% minion attack/cast speed, life and all elemental resistance.
- **Method:** Fracture increased effect or T1 life. Harvest reforge speed until T1 minion attack/cast speed, annul down to those two desired affixes, then Allflame Exalt twice for the remaining affixes.
- **Retry/scope:** Lost speed returns to reforge speed. Bad Exalts require deciding whether to rescue or reroll; no guaranteed recovery is supplied. Choose an item level that supports all targets; the answer does not specify it. No easy deterministic completion is claimed.
- **Source:** [Request and answer][329-120].

### 12. Memory-strand critical-slam helmet

- **Start/goal:** Giantslayer Helmet with high memory strands; desirable Infamous critical-slam modifier plus useful ordinary affixes.
- **Method:** Cyaxan's Ducat, selecting the best Infamous outcome. Retry poor results while preserving strands where possible. Allflame ordinary Exalt to six mods, then decide whether an ordinary or Allflame Unravelling Orb is worth using.
- **Retry/scope:** Sirgog's contributed recipe, acknowledged by Wuslwiz. Suggested bias toward the critical-slam mod is explicitly speculation. Exact final affixes are flexible; historical base prices and resale claims are omitted.
- **Source:** [Sirgog's recipe][329-130].

### 13. +6 lightning critical staff

- **Start/goal:** Staff with fractured +4 lightning spell gems; +2 all spell gems, optional good spell damage, cast speed/critical modifiers.
- **Method:** Aetheric + Corroded + Shuddering until +2 all spells. Lock prefixes, Harvest reforge caster until a good solo spell-critical or cast-speed suffix. Lock prefixes, Allflame Veiled Exalt while retaining that suffix if offered. Block damage-per-charge before unveiling critical, cast speed or double damage; craft the last suffix.
- **Retry/scope:** Reforge suffixes again if the useful suffix is lost. Prefix/suffix slot cleanup is required between attempts. The cold-staff discussion recommends fracturing the elemental gem mod first, but does not establish every elemental variant's fossil pool.
- **Source:** [Answer][329-141].

### 14. Transfer Breach global defences to Twilight Regalia

- **Start/goal:** Breach global-defences chest donor and high-level Twilight Regalia; transfer the global-defences prefix to the regalia.
- **Method:** Bring the donor to four affixes and use Ducat of Kishara to isolate global defences. On the regalia, low-tier flat armour/evasion essence, then isolate that non-native prefix. Unpredictably recombine, aiming to choose the regalia while transferring global defences.
- **Retry/scope:** The answer claims guaranteed transfer conditional on choosing the right base; this is not verified here. If the donor survives with its wanted mod, retry with another regalia. This produces a crafting base, not finished high ES gear.
- **Source:** [Answer][329-153].

### 15. Rescue a temple suffix after a failed fracture

- **Start/goal:** Failed-fracture temple gloves and another compatible base; recover the unfractured temple suffix onto a fresh base.
- **Method:** Isolate the temple modifier as the donor's only suffix. On the recipient, isolate an essence attribute suffix that is genuinely non-native to the relevant bases. Unpredictably recombine. Repeat as needed to move away from the fractured base, then fill prefixes to regain a four-mod fracture candidate.
- **Retry/scope:** Check native attribute pools for both bases. The author corrected an initial dexterity example because all gloves can naturally roll dexterity; do not reproduce that example or infer universal guaranteed transfer. Supporting base-preparation method for 02.
- **Source:** [Edited answer and correction][329-165].

### 16. High-armour chest from Dread essences

- **Start/goal:** Armour chest; essence flat armour, high increased armour, hybrid armour or high life and useful suffixes.
- **Method:** Deafening Dread until high increased armour. Annul/beast craft to open a prefix and suffix space. Allflame Exalt for hybrid armour/high life, then finish with more Exalts and a bench craft.
- **Retry/scope:** Cleanup can destroy wanted armour rolls; restart or settle. Foulborn Exalt was proposed early, but the same answer advises ordinary Exalts at the later prices. Separate armour analogue of 01.
- **Source:** [Answer][329-173].

### 17. Budget ES armour without Allflame

- **Start/goal:** ES chest, helmet or gloves; Woe flat ES, acceptable hybrid ES, crafted increased ES and usable suffixes.
- **Method:** Woe essence until a decent hybrid ES roll. Open the last prefix with annul or prefix-to-suffix beast craft. Bench craft increased ES and Exalt remaining suffixes.
- **Retry/scope:** Restart if cleanup removes a desired ES roll. Unlike 01, this targets hybrid ES during essence rolling and uses crafted increased ES. For boots the answer instead suggests movement speed alongside flat ES and crafted increased ES. Related family in [cheap-high-es-regalia-fractured.toml](../fixtures/guides/cheap-high-es-regalia-fractured.toml).
- **Source:** [Answer][329-180].

### 18. +2 minion prefix onto Archdemon Crown

- **Start/goal:** Any +2 minion helmet donor and an Archdemon Crown; move +2 minions onto the crown, then add general life/resistances.
- **Method:** Reduce the donor to four affixes. Allflame Ducat of Kishara to isolate +2 minions. On the crown, apply a low flat-evasion essence and isolate it as the only prefix. Unpredictably recombine and aim for the crown. Finish through Allflame or further recombination.
- **Retry/scope:** Replace failed recipients/donors as needed. The conditional transfer claim depends on the non-native prefix setup. No exact six-affix finish is provided; the transfer itself is the recipe's target.
- **Source:** [Answer][329-186].

### 19. RF helmet: Burning Damage plus Horror

- **Start/goal:** Elder-compatible helmet with level 20 Burning Damage support and Horror's socketed-skill elemental damage modifier.
- **Method:** Alteration-roll Burning Damage on one donor. Apply Horror once to another and isolate the essence suffix with annuls/beasts. Unpredictably recombine until both survive on a valid base. Add useful affixes with Allflame Exalts and a bench craft.
- **Retry/scope:** Rebuild failed donors; verify the chosen output base can retain the influenced modifier. The follow-up about Elder donors receives an unrelated ring answer, so it is not evidence about influence requirements. Related to [eight-link-archdemon.toml](../fixtures/guides/eight-link-archdemon.toml), with a smaller target and different construction.
- **Source:** [Answer][329-202].

### 20. +2 minion wand with native minion damage

- **Start/goal:** Non-fractured, non-influenced minion wand; native increased minion damage, +1 all spells and +1 minions.
- **Method:** Recombine native minion damage with +1 all spells to make one two-prefix donor. Make another with native minion damage and +1 minions. Recombine the overlapping pairs until all three prefixes survive.
- **Retry/scope:** Rebuild/recycle failed pairs. The answer deliberately uses natural minion damage; it does not establish a recipe duplicating the exclusive Fear essence mod across donors. Suffix finishing is outside this answer. Alternative to the fossil prefix method in [minion-plus-two-wand.toml](../fixtures/guides/minion-plus-two-wand.toml).
- **Source:** [Answer][329-217].

### 21. Flame Link elemental critical foil

- **Start/goal:** Foil with fractured increased elemental damage with attack skills; attack speed, critical chance/multiplier, elemental penetration and crafted fire gained as chaos.
- **Method:** Attack-speed essence until a T1 critical suffix with free suffix/prefix. Lock suffixes, reforge critical for the other critical modifier. Matron/lock, Veiled Chaos, block increased physical damage, unveil elemental penetration and craft fire gained as chaos.
- **Retry/scope:** Bad critical tier restarts essence rolling. Failed prefix unveils may require cleanup or restart; Veiled Chaos can fill slots. The follow-up confirms the fractured modifier is elemental damage with attacks, not increased fire damage. A similar dagger version is also supplied in 3.27.
- **Source:** [Answer and clarification][329-224]; [dagger variant][327-283].

### 22. Redeemer cold-damage Cryonic Ring

- **Start/goal:** Redeemer Cryonic Ring; cold damage against chilled enemies and cold damage per frenzy charge as prefixes.
- **Method:** Isolate one desired Redeemer prefix on each of two donors, then `1P + 1P → 2P` recombine until both survive. Preserve both prefixes for subsequent suffix crafting.
- **Retry/scope:** Rebuild/recycle donors on failure. The answer supplies only the prefix construction; exact suffix targets and a finishing loop are unspecified.
- **Source:** [Answer][329-230].

### 23. Power-charge helmet with life

- **Start/goal:** Warlord helmet; +1 maximum power charge and T1 life prefixes, chaos resistance and useful remaining affixes.
- **Method:** Alteration-roll power charge and T1 life on separate Warlord donors. Recombine into two prefixes. Lock prefixes, reforge chaos, then Allflame Exalt the remaining mods.
- **Retry/scope:** Rebuild failed prefixes; preserve them during suffix cleanup. The answer does not guarantee the request's optional suppression/two elemental resistances. Do not treat the flexible finish as a five-specific-mod target.
- **Source:** [Answer][329-240].

### 24. Strength critical-scaling and intelligence helmet

- **Start/goal:** Dual-influenced helmet with critical chance per strength, increased intelligence, critical multiplier, physical damage taken as fire and crafted life.
- **Method:** Awakener Orb the two desired suffixes. Annul to open a suffix/prefix, bench lock suffixes, reforge critical for multiplier. With suffixes full, Matron/lock and reforge fire or physical for physical taken as fire. Craft life, optionally Allflame Exalt another prefix.
- **Retry/scope:** Failed Awakener cleanup can lose a wanted suffix. Prefix reforges can fill all slots; the answer explicitly allows bricking. Exact influence donors/base and mod tiers must be resolved before encoding.
- **Source:** [Answer][329-257].

### 25. High-block, high-ES recovery shield

- **Start/goal:** ES shield; flat ES, increased ES and increased block prefixes, ES recovery on block suffix.
- **Method:** Overlapping two-prefix recombinations into three prefixes. Add Shaper influence with a Shaper Exalt. Lock prefixes, Harvest reforge defence until recovery on block. Allflame Exalt remaining suffixes.
- **Retry/scope:** Reforges may fill suffixes; annul or rebuild if no lock can be applied. The requested additional spell-critical suffix is not deterministic and is not part of the supplied guaranteed route. Substantial overlap with [es-block-shield.toml](../fixtures/guides/es-block-shield.toml); the AMA adds an Allflame finish.
- **Source:** [Answer][329-279].

### 26. Strength-stacking dual-influenced talisman

- **Start/goal:** Talisman using the special beastcraft/imprint rules discussed in 3.29; percentage attributes/strength, a high flat attribute, damage per strength, life and a useful final prefix.
- **Method:** Make the talisman Shaper/Warlord, roll one desired percentage-attribute suffix and Awakener Orb the other from a donor, with an imprint for recovery. Rare-imprint the two-suffix stage, then lock/reforge attributes or remove-prefix/add-suffix beast craft for the flat attribute. Clear prefixes, imprint, Exalt for damage per strength, isolate it with imprint-backed annuls. Lock suffixes, imprint, augment life until T1, then imprint/Allflame Exalt a final prefix.
- **Retry/scope:** Each branch depends on the correct talisman-specific imprint being available; restore and re-imprint between failures. Ordinary imprint rules do not establish this method. Keep as a patch-specific candidate, pending verification of the special rollback through Allflame and influence operations.
- **Source:** [Detailed answer][329-314].

### 27. Despair-on-hit DoT ring

- **Start/goal:** Delve ring with fractured Despair on hit; essence DoT multiplier, resistance, life and crafted mana cost.
- **Method:** DoT essence until high resistance and an open prefix. With suffixes full, Matron/lock and Harvest reforge life until good life with a spare prefix. Craft mana cost.
- **Retry/scope:** Reapply lock before each life roll; full prefixes require annul cleanup or settling. The answer suggests recombination as an alternative to the fractured Delve base but supplies no specific ladder for that alternative.
- **Source:** [Answer][329-318].

### 28. Life, attack-speed and AoE gloves

- **Start/goal:** Preferably fractured-life gloves; two resistances, attack speed, unveiled area damage/AoE and crafted damage during flask effect.
- **Method:** Resistance essence until a second resistance with an open suffix/prefix. Bench lock suffixes, reforge speed for attack speed. Matron/lock, Veiled Chaos, block mana and unveil AoE/area damage. Craft flask-effect damage.
- **Retry/scope:** Use eligible Eldritch prefix annuls to retry failed unveils while retaining suffixes. Without fractured life, first establish life as the desired prefix with Exarch-dominant currency. An open slot must remain for the final craft.
- **Source:** [Answer][329-320].

## 3.28: additional goals and methods

### 29. RF jewel with maximum fire resistance

- **Start/goal:** Ordinary jewel with fractured +2 maximum fire resistance; life and fire DoT multiplier.
- **Method:** Harvest swap another elemental maximum-resistance jewel to fire if necessary, fracture that modifier or buy the fractured base, then reforge fire until life and fire DoT multiplier. Exalt an open prefix if useful.
- **Retry/scope:** Failed fracture replaces the starting base; otherwise repeat reforges. The quoted roughly 300 reforges is a historical estimate, not a current probability/cost guarantee. Adds a jewel family absent from the armour/weapon-heavy guide set.
- **Source:** [Answer][328-146].

### 30. Suppression chest with strong defence prefixes

- **Start/goal:** Suppression-compatible chest; suppression plus high defence prefixes. Specific suffixes/defence type are left flexible.
- **Method:** Either fracture suppression and Dense fossil for triple defence, or start with fractured flat defence, essence desired suffixes until suppression, then Exarch-dominant Eldritch prefix work for hybrid defence and bench increased defence.
- **Retry/scope:** Repeat fossils/essences; only use Eldritch currency on eligible bases. The two branches trade a fixed suffix against a fixed prefix. Related to [necrotic-dmg-shocked.toml](../fixtures/guides/necrotic-dmg-shocked.toml), but without its specific essence target.
- **Source:** [Answer][328-180].

### 31. +1 chaos, block and ES-recovery shield

- **Start/goal:** Shaper intelligence shield; increased block prefix, ES recovery on block suffix and +1 chaos gems prefix.
- **Method:** Alteration-roll block and ES recovery on separate donors, recombine to a two-mod rare. Multimod + Prefixes Cannot Be Changed + Suffixes Cannot Be Changed, leaving exactly one prefix open and all suffixes full. Reforge chaos for +1 chaos gems. Remove crafts and multimod desired finishing affixes.
- **Retry/scope:** Rebuild failed donor combinations. Check this exact slot arrangement before the reforge; the locks protect both desired affixes while allowing the last prefix. Related shield construction to [plus-one-phys-spell-crit-shield.toml](../fixtures/guides/plus-one-phys-spell-crit-shield.toml), with different targets/tags.
- **Source:** [Answer][328-184].

### 32. +1 minion, minion-damage and life-recovery shield

- **Start/goal:** Shaper-compatible minion shield; natural minion damage prefix, life recovery on block suffix and +1 minions prefix.
- **Method:** Roll/isolate the influenced recovery suffix and high minion-damage prefix on donors. Recombine into a two-mod rare. Multimod + both affix locks so exactly one prefix is open and suffixes are full. Harvest reforge minion for +1 minions. Remove crafts and multimod the finish.
- **Retry/scope:** Rebuild failed combinations. Confirm the output base can roll the desired minion prefix; this route uses influence rather than fracture. The requester later reports successful use of the answer.
- **Source:** [Answer][328-188].

### 33. Crusader physical DoT Citadel Bow

- **Start/goal:** High-quality, sufficiently high-level Crusader Citadel Bow; essence flat physical, generic DoT suffix, hybrid physical/projectile-speed prefix and crafted physical scaling.
- **Method:** Contempt essence until DoT multiplier; annul to exactly those two mods. Multimod + both affix locks, leaving only one prefix open. Reforge speed for the Crusader hybrid prefix. Remove crafts and multimod increased physical damage plus attack speed or physical DoT multiplier.
- **Retry/scope:** Failed isolation restarts essence rolling; the exact protected slot setup is necessary for the tagged reforge. Tempering is optional and outside the core recipe. A new bow goal rather than the existing elemental-bow guides.
- **Source:** [Answer][328-199].

### 34. Low-life spell-critical and Concentrated Effect helmet

- **Start/goal:** Shaper/Elder helmet; Hysteria low-life damage suffix, spell-critical suffix, Concentrated Effect prefix and crafted/unveiled AoE gem levels.
- **Method:** Hysteria essence until desired influenced critical suffix. Fill/protect desired suffixes, lock and scour prefixes. Exalt twice for Concentrated Effect; lock/scour and retry if missed. Either bench craft AoE gems or lock suffixes/Veiled Exalt, block zombie/skeleton levels if there is room, and seek +2 AoE.
- **Retry/scope:** The premium unveil can remove Concentrated Effect. Source gives a rough path: specify any third suffix before using Matron or treating suffixes as full. Related to [eight-link-archdemon.toml](../fixtures/guides/eight-link-archdemon.toml), with a different essence goal.
- **Source:** [Answer][328-230].

### 35. Strength and unveiled strike-range gloves

- **Start/goal:** Fractured-life gloves; essence strength, elemental resistance, attack speed and unveiled melee strike range.
- **Method:** Strength essence until resistance with open suffix/prefix. Bench lock suffixes, reforge speed. Open a prefix with Exarch-dominant Eldritch annul if necessary. Matron/lock, Veiled Chaos, block mana and unveil strike range. Craft the remaining prefix.
- **Retry/scope:** Eldritch-annul the unwanted unveil and repeat the lock/veil stage if possible. This is the ordinary unveiled strike-range mod, distinct from the double-elevated Warlord/Elder goal in [strike-range-gloves.toml](../fixtures/guides/strike-range-gloves.toml).
- **Source:** [Answer][328-235].

### 36. Budget mana-RF sceptre

- **Start/goal:** Sceptre with natural mana prefix and generic DoT multiplier suffix; crafted fire DoT multiplier and spell damage.
- **Method:** Alteration-roll solo mana on one donor and solo generic DoT multiplier on another. Recombine until both survive, then multimod fire DoT multiplier and spell damage.
- **Retry/scope:** Rebuild/recycle failed pairs. The answer distinguishes this from a much more expensive two-prefix/two-suffix item with natural double multipliers. Exact base and tiers are unspecified.
- **Source:** [Answer][328-271].

### 37. Elevated strength-stacking boots

- **Start/goal:** Influenced boots; flat strength, elevated percentage strength, life regeneration rate and high life/movement speed prefixes.
- **Method:** Recombine strength + elevated percentage strength on one donor, strength + regeneration on another, then combine for three suffixes. Lock suffixes/scour prefixes. Exalt twice looking for high life or movement speed, clearing/retrying prefixes if missed. Bench the missing prefix. Premium alternative: isolate life, lock/Veiled Exalt, block mana and unveil movement speed.
- **Retry/scope:** Suffix construction is a long RNG ladder. Premium prefix finishing risks removing life. The requested additional resistance is not retained as a guaranteed fourth suffix on ordinary boots.
- **Source:** [Answer][328-296].

### 38. Reservation and life-recovery Shaper shield

- **Start/goal:** Armour/evasion Shaper shield; reservation, life on block, chaos resistance, life and crafted block.
- **Method:** Alteration-roll recovery and reservation on separate donors. Recombine both suffixes. Bench lock suffixes and reforge chaos; with all suffixes filled, Matron/lock and reforge life until high life. Craft increased block.
- **Retry/scope:** Full prefixes may force annul. The answer warns that pure intelligence bases add +1 chaos gems to the relevant pool. Check other influenced chaos outcomes too. Related to [totem-reservation-recover-life-shield.toml](../fixtures/guides/totem-reservation-recover-life-shield.toml), without the totem prefix.
- **Source:** [Answer][328-304].

### 39. Double-DoT Hunter quiver

- **Start/goal:** Hunter quiver; chaos DoT multiplier and DoT multiplier with bow skills suffixes, life/bow-damage prefixes and crafted attack speed.
- **Method:** Alteration-roll each desired suffix separately and recombine into a two-mod rare. Craft Cannot Roll Attack Modifiers. Exalt prefixes; if unsatisfied, lock suffixes/scour, recraft Cannot Roll Attack Modifiers and retry. Fill prefixes, remove the blocker and craft attack speed.
- **Retry/scope:** Influenced prefixes can appear alongside life and bow damage; high tiers are not guaranteed. Reserve the suffix slot needed for blocker/finishing craft. Do not turn the historical expected outcomes into a deterministic three-prefix goal.
- **Source:** [Answer][328-322].

### 40. Holy Relic trigger sword with +1 socketed gems

- **Start/goal:** Non-influenced thrusting sword; essence minion damage, attack speed, +1 socketed gems, crafted trigger and Hits Cannot Be Evaded.
- **Method:** Fear essence until attack speed. Annul/split to isolate minion damage and speed. Multimod + Cannot Roll Attack Modifiers to fill suffixes, then Exalt for +1 socketed gems as the only eligible non-attack prefix. Remove crafts and multimod trigger plus Hits Cannot Be Evaded.
- **Retry/scope:** Failed isolation restarts essences. The non-influenced starting condition and full suffixes are essential. This is different from the snapshot-specific weapon in [minion-pact-snapshot-weapon.toml](../fixtures/guides/minion-pact-snapshot-weapon.toml).
- **Source:** [Answer][328-426].

### 41. +2 physical spell dagger with double damage

- **Start/goal:** Caster-compatible dagger; spell damage, +1 all spells and +1 physical spells prefixes, unveiled double damage and a crafted suffix.
- **Method:** Recombine isolated spell damage and +1 all spells. Clear suffixes safely. Multimod + Prefixes Cannot Be Changed + Cannot Roll Attack Modifiers, then Harvest augment physical for +1 physical gems. Restore prefix lock if removed. Veiled Chaos, block damage-per-charge, unveil double damage and craft the last mod.
- **Retry/scope:** Failed unveils return to safe suffix clearing. Check attack blockers and the dagger's gem pool; not every dagger base has caster mods. Broader goal family in [dagger-plus-two-explode-synth.toml](../fixtures/guides/dagger-plus-two-explode-synth.toml), with different starting conditions.
- **Source:** [Answer][328-463].

## 3.27: jewellery, jewels and minion equipment

### 42. +2 minion suppression helmet

- **Start/goal:** Fractured +2 minion helmet on a suppression-compatible base; elemental resistance, suppression, chaos resistance and life.
- **Method:** Produce +2 donors with Rog and fracture, or buy the base. Resistance essence until suppression with open suffix/prefix. Bench lock suffixes, reforge chaos. Make Exarch dominant, roll prefixes for life and craft defence/life hybrid.
- **Retry/scope:** Bad chaos tiers require rebuilding suffixes; failed fractures need another base. The historical Genesis Tree source of fractures is excluded from the portable route. Related to [plus-two-minion-reservation-helmet.toml](../fixtures/guides/plus-two-minion-reservation-helmet.toml), replacing reservation with suppression/resistances.
- **Source:** [Answer][327-43].

### 43. Synthesized strength-stacking chaos attack wand

- **Start/goal:** Fire-per-strength synthesized wand; critical chance, critical multiplier, attack speed, flat chaos, unveiled chaos penetration and crafted spell damage.
- **Method:** Critical/speed essence until another desired critical suffix and open suffix/prefix. Bench lock suffixes, reforge critical for the third suffix. Lock/scour prefixes, lock again and reforge/augment chaos for flat chaos. Lock/Veiled Exalt, block increased physical damage, unveil chaos penetration and craft spell damage.
- **Retry/scope:** Attack critical versus spell critical is a roll; restart suffixes when wrong. Chaos can produce +1 chaos gems instead of flat chaos. Veiled Exalt can remove the flat damage. The implicit is a purchased prerequisite, not produced by this recipe.
- **Source:** [Answer][327-91].

### 44. Minion physical-damage and attack-speed Ghastly Eye

- **Start/goal:** Ghastly Eye Jewel; flat physical damage for minions and minion attack speed.
- **Method:** Corroded + Jagged + Shuddering in a three-socket resonator. If T1 physical damage is essential, fracture that prefix first and Harvest reforge speed for the suffix instead.
- **Retry/scope:** Repeat fossils/Harvest; the fossil route does not guarantee tiers or the remaining jewel affixes. The stated attempt count is historical. A new abyss-jewel family.
- **Source:** [Answer][327-152].

### 45. ES and critical-multiplier jewel

- **Start/goal:** Ordinary jewel with fractured increased maximum ES; multiple applicable spell/fire critical multipliers.
- **Method:** Deft + Lucent or Deft + Opulent in a two-socket resonator, as suggested in the answer.
- **Retry/scope:** Repeat fossils; choose a concrete set of mutually compatible multipliers and check weights before encoding. The answer is a recipe family and its roughly 100-attempt claim is not validated. No fixed three-multiplier target is inferred from the request.
- **Source:** [Answer][327-188].

### 46. Dexterity, resistance and unveiled-life ring

- **Start/goal:** Ring, such as Unset; dexterity, elemental resistance, chaos resistance, unveiled life and a crafted finishing prefix.
- **Method:** Dexterity essence until good elemental resistance with open suffix/prefix. Bench lock suffixes and reforge chaos. Matron/lock, Veiled Chaos, block mana, unveil flat life and craft another prefix.
- **Retry/scope:** Bad chaos tier restarts suffixes; failed veils need safe cleanup or restart. No Eldritch recovery is available on rings. This is a generic jewellery recipe distinct from the guide set's specialized Cogwork/amulet goals.
- **Source:** [Answer][327-191].

### 47. Elevated spectre-level suppression boots

- **Start/goal:** Elder boots; elevated spectre gem-level suffix, suppression, optional chaos resistance and unveiled movement speed.
- **Method:** Recombine the elevated Elder suffix with suppression. If the third suffix is open, bench lock suffixes/reforge chaos. With suffixes completed, Matron/lock, Veiled Chaos, block mana and unveil movement speed. Bench the last prefix if available.
- **Retry/scope:** Failed recombination is costly; rebuild donor stages. Elevating the initial donor is a prerequisite not expanded in this answer. Related to [tailwind-plus-two-spectre-boots.toml](../fixtures/guides/tailwind-plus-two-spectre-boots.toml), with suppression instead of Tailwind.
- **Source:** [Answer][327-194].

### 48. Double-DoT and spell-critical fire sceptre

- **Start/goal:** Sceptre; generic DoT multiplier, fire DoT multiplier and spell-critical suffixes, unveiled increased fire damage and crafted DoT damage.
- **Method:** Shrieking spell-critical essence until generic DoT multiplier for one pair. Combine a separate spell-critical donor with isolated high fire DoT multiplier for another pair. Recombine overlapping pairs for three suffixes. Matron/lock, Veiled Chaos, block physical damage, unveil increased fire damage, then craft DoT damage.
- **Retry/scope:** Rebuild failed suffix pairs. The fire unveil is uncommon and may need repeated protected prefix attempts; full prefixes can require cleanup. Exact sceptre base/tiers were not supplied in text.
- **Source:** [Answer][327-203].

### 49. Strength, intelligence and percentage-attribute belt

- **Start/goal:** Initially non-influenced belt; intelligence, T1 strength, Hunter percentage attributes, unveiled life and crafted elemental attack damage.
- **Method:** Intelligence essence until T1 strength, two filled prefixes, one open prefix and one open suffix. Hunter Exalt for percentage attributes. With suffixes full, Matron/lock and Veiled Chaos; block mana, unveil life, craft elemental damage with attacks.
- **Retry/scope:** Failed Hunter slam means another base or a different project; do not keep using the pre-influence route on it. Veiled prefix rolls can fill slots. High attribute tiers and an available finishing prefix are part of acceptance, not guaranteed by the slam.
- **Source:** [Answer][327-297].

### 50. Doryani's Lesson cold cluster

- **Start/goal:** Large cold cluster; Doryani's Lesson, Blanketed Snow and Prismatic Heart.
- **Method:** Fracture Doryani's Lesson, then Dense + Prismatic + Pristine in a three-socket resonator for the other two notables.
- **Retry/scope:** Failed fracture needs another base; otherwise repeat fossils. Verify the cluster's passive count/item level before creating an exact goal. The historical attempt count and market explanation are omitted from cost inputs. This gives a concrete target missing from 03's general elemental-cluster method.
- **Source:** [Request and answer][327-330].

### 51. Synthesized frenzy-cold ring with attributes

- **Start/goal:** Synthesized ring; Horror cold damage per frenzy prefix, strength/intelligence suffixes, chaos resistance and life.
- **Method:** Fundamental + Glyphic + Sanctified until the essence prefix and desired attributes. With one suffix/prefix open, lock suffixes and augment chaos. Lock suffixes, Veiled Exalt, block mana, unveil life and craft a final mod.
- **Retry/scope:** Both augment and Veiled Exalt can remove the essence prefix; failed attempts may require repeating fossils. It is not a deterministic four-target craft. Glyphic also has alternate essence outcomes. The requested synthesized implicit is a starting-base condition.
- **Source:** [Answer][327-306].

## 3.26: older recipes still useful as candidates

These methods are historical candidates for the 3.29 target. Recheck weapon affix position, mod pools and influence rules against the pinned game data before promoting them. Later AMAs explicitly refer readers back to the two-handed axe method, but that does not independently validate every older recipe.

### 52. Elder/Warlord culling physical Despot Axe

- **Start/goal:** Item-level 71–72 Despot Axe with Elder + Warlord influence; essence flat physical, Elder physical/culling suffix, Warlord hybrid physical/attack-speed prefix and crafted physical/speed.
- **Method:** Make the dual-influenced base using influenced Exalts, Harvest influence swaps and Awakener Orb. Contempt essence until the Elder culling suffix, then isolate it with the essence prefix. Multimod + both affix locks, leaving exactly one prefix open. Harvest reforge speed for the Warlord hybrid prefix. Remove crafts and multimod physical damage plus attack speed.
- **Retry/scope:** Isolation is the main restart point. Low item level intentionally narrows the pool; do not silently substitute level 86. Related physical-weapon family in [cheap-phys-weapon.toml](../fixtures/guides/cheap-phys-weapon.toml), with different influenced targets.
- **Source:** [Opening post][ama326].

### 53. Triple-element Pneumatic Dagger

- **Start/goal:** Pneumatic Dagger; three strong flat elemental prefixes and two crafted suffixes.
- **Method:** Flat-element essence until a second good flat element. Annul to four affixes and fracture, accepting a hit on either wanted elemental prefix. Essence a different element until the third is good. Lock prefixes/scour suffixes, then multimod two chosen suffixes.
- **Retry/scope:** Failed isolation/fracture needs another candidate. Triple natural T1 is not guaranteed: one roll is essence-derived. Compare the value of a premium suffix finish separately. Different base/goal from the guide set's elemental bows and spell wands.
- **Source:** [Answer][326-26].

### 54. Overlord staff cluster

- **Start/goal:** Staff large cluster; Overlord with Fuel the Fight, optionally Martial Prowess.
- **Method:** Bulk Harvest reforge attack, keeping Overlord with desirable additional notables. The answer describes Overlord plus one/two useful notables as saleable intermediate outcomes.
- **Retry/scope:** Keep rolling until the chosen exact target is met; Overlord alone is not acceptance for the requested three-notable jewel. Passive count/item level and the exact notable combination require specification. No historical profit margins or suggested automation practices are included.
- **Source:** [Answer][326-46].

### 55. Fractured-spell-damage +2 physical wand

- **Start/goal:** Caster wand with fractured increased spell damage; +1 all spells, +1 physical spells and crafted cast-speed/critical suffixes.
- **Method:** Corroded + Metallic + Jagged + Shuddering until +1 all spells and an open suffix. Lock prefixes/scour. Multimod + Prefixes Cannot Be Changed + Cannot Roll Attack Modifiers, then augment physical for +1 physical gems. Remove unwanted crafted locks while retaining the wanted prefixes; multimod cast speed and another chosen suffix.
- **Retry/scope:** Failed fossil cleanup restarts fossils. Do not annul a surviving prefix-lock/multimod setup without accounting for removable crafts and the fractured prefix. Compare with [plus-two-physdot-wand.toml](../fixtures/guides/plus-two-physdot-wand.toml), which targets a different finish.
- **Source:** [Answer][326-49].

### 56. Lightning and fire-DoT quiver

- **Start/goal:** Initially non-influenced quiver; Torment flat lightning prefix, DoT with bow skills suffix, life/bow damage, crafted attack speed and Hunter fire DoT multiplier.
- **Method:** Torment essence until generic bow DoT multiplier; isolate the two. Multimod + Cannot Roll Attack Modifiers, Exalt twice for life/bow damage under the source's pool assumptions. Remove crafts, craft attack speed, then Hunter Exalt for the desired influenced suffix.
- **Retry/scope:** Wrong Hunter result is a failed target, potentially saleable rather than repairable. Check the quiver's non-attack pool in current data; the influence slam does not guarantee fire DoT multiplier or good tiers.
- **Source:** [Answer][326-82].

### 57. Reservation, physical reduction and critical reduction shield

- **Start/goal:** Level 86 initially non-influenced shield; physical reduction, Shaper reservation, reduced extra critical damage, life and crafted support-gem levels.
- **Method:** Reforge physical for high physical reduction with two filled prefixes, one open prefix and two open suffixes. Fill the final prefix with a bench craft, then Shaper Exalt for reservation. Recombine failed bases as needed to get both desired suffixes. Remove the prefix craft, bench lock suffixes/reforge critical. With suffixes full, Matron/lock/reforge life, then craft +2 support gems.
- **Retry/scope:** Wrong influence slams require new/recombined bases; filled prefixes during life rolling need cleanup. This is the author's corrected recipe. Related to [reservation-life-max-fire-shield.toml](../fixtures/guides/reservation-life-max-fire-shield.toml), replacing maximum fire resistance with physical/critical reduction.
- **Source:** [Edited answer][326-107].

### 58. Archmage mana and spell-damage wand

- **Start/goal:** Wand with fractured spell-damage/mana hybrid; Woe spell damage, high mana, cast speed and unveiled/crafted double damage variants.
- **Method:** Woe essence until acceptable mana. Open a suffix, lock prefixes/reforge caster until good cast speed. Lock prefixes/Veiled Exalt, block damage-per-charge, unveil double damage, craft the other double-damage option.
- **Retry/scope:** The source recommends settling below T1 mana/cast speed for cost. Reforges can fill suffixes; Veiled Exalt can delete cast speed. Natural and crafted double-damage variants must be checked for compatibility. No current cost estimate is supplied.
- **Source:** [Answer][326-287].

### 59. Intelligence, strength and cast-speed ring

- **Start/goal:** Ring or Cogwork Ring; intelligence, strength, cast speed, ES and crafted channelled-skill cost.
- **Method:** Intelligence essence until high strength. Bench lock suffixes/reforge caster for cast speed. On Cogwork, optionally fill the fourth suffix after blocking prefixes. Clear prefixes with suffix lock/scour, relock and augment defence for ES. Craft channelled mana cost.
- **Retry/scope:** ES tier is random; clear/retry prefixes if unsuitable. The extra Cogwork suffix is random, not guaranteed all resistance. Exact base and occupied slots alter what can be blocked and must be specified before encoding.
- **Source:** [Answer][326-210].

### 60. Regeneration, suppression and chaos-resistance boots

- **Start/goal:** Boots with fractured regeneration suffix; chaos resistance, suppression and unveiled movement speed.
- **Method:** Chaos-resistance essence until suppression. Apply an Ember for Exarch dominance and Eldritch-annul an available prefix if needed. Matron/lock suffixes, Veiled Chaos, block life and unveil movement speed.
- **Retry/scope:** Veiled Chaos may fill prefixes before the blocker can be crafted; preserve suffixes during subsequent prefix cleanup. Exact regeneration mod/tier should be taken from the starting base. Related to [int-chaos-armour-boots.toml](../fixtures/guides/int-chaos-armour-boots.toml), with different fixed/essence suffixes.
- **Source:** [Answer][326-344].

## Answers held for review

These are useful leads but are not counted among the 60 catalog entries.

- **+2 physical DoT wand, 3.29:** The answer gives Corroded + Metallic + Shuddering, Cannot Roll Attack Modifiers, augment physical, then veiled physical DoT multiplier and crafted cast speed. The text only says to start with a fractured base; the fracture is in an unrecovered item image. Preserve the stated prefix-removal risk, and resolve the fracture before encoding. [Answer][329-77].
- **Double-elevated explode/+1 strength high-ES chest:** Detailed recombination, Dominance, lock and veiled-suffix sequence, but duplicated answers and a later correction change the elevation step. The lock/preview retry sequence is underspecified and should not be modeled as an automatic undo. Needs an exact influenced goal and full failure-state audit. [Answer][329-135]; [elevation correction][329-138].
- **Abyssal-socket ES boots:** Starting socket fracture is clear, but the special essence modifier and other targets depend on an unrecovered image. Do not guess which essence from “special essence mod.” [Answer][329-298].
- **+2 cold spell-critical weapon:** The answer describes +1 all/+1 cold paired with spell critical, then recombination and multimod. The missing request/image leaves the exact item type/base unclear; the +1 cold vendor recipe must be matched to the actual weapon. [Answer][329-304].
- **Chaos DoT/generic DoT wand:** The author proposes recombination or fractured chaos DoT plus cast-speed essences, and explicitly warns that the suffix-first fractured method only permits +1, not +2. Exact requested gem/base/tier goals are missing. [Answer][329-301].
- **Minion life-fracture Bone Ring:** The 3.28 Bound + Corroded + Dense → reforge chaos → reforge minion method is recoverable, but the 3.29 author endorses minion-damage fracture instead. Keep it as an alternative for later price comparison, not a second default recipe. [Older method][328-249]; [newer endorsement][329-105].

The Dread Captain's Cutlass request received no recipe: the author explicitly declined to give untested advice. Answers that only said “look at older AMAs,” requests with no recovered answer, and ordinary build/market discussions were not turned into invented recipes.

## Suggested next fixture additions

Start with exact goals that add coverage beyond the existing 46 methods: RF maximum-resistance jewel (29), cold three-notable cluster (50), minion attack-speed Ghastly Eye (44), Holy Relic trigger sword (40), Crusader physical bow (33), influenced minion shield (32), and Elder/Warlord culling axe (52). Then add Allflame alternatives for existing ES goals (01–02) once preview choice, sulphur and intangibility are represented. This prioritization is an editorial assessment of coverage, not an assertion that any recipe currently passes the simulator.

For promotion into `fixtures/guides`, resolve exact base/ilvl/mod templates, state the terminal goal and any bought-stage assumptions, encode every retry and brick state, and validate against the pinned 3.29 snapshot. The 60 entries above deliberately retain partial/flexible targets where the source does.

[ama329]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/ama_crafting_ama_for_329_curse_of_the_allflame/?sort=top
[ama328]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/
[ama327]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/ama_crafting_ama_for_327_keepers_of_the_flame/
[ama326]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/ama_crafting_ama_for_326_secrets_of_the_atlas/
[329-233]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yyxjj/
[329-161]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ygt4c/
[329-253]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p124lyb/
[329-31]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ykjgw/
[329-46]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ye0wm/
[329-49]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p103dyh/
[329-85]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yo39k/
[329-96]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ygb13/
[329-102]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0z1u1h/
[329-105]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p129dja/
[328-249]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oae9i1n/
[329-111]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yul0j/
[329-120]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p129unc/
[329-130]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p101chd/
[329-141]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p117r8g/
[329-153]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ymj90/
[329-165]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ycp08/
[329-173]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0y90nb/
[329-180]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0y7v5r/
[329-186]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yx65e/
[329-202]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0ysjar/
[329-217]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0z0ti5/
[329-224]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yzuru/
[327-283]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxvd0k/
[329-230]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yz4er/
[329-240]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p12ajxv/
[329-257]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p1248x3/
[329-279]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p12b8ce/
[329-314]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11wgmn/
[329-318]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11eqmy/
[329-320]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11e5rz/
[328-146]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oae1vel/
[328-180]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oadxv50/
[328-184]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oae6mtb/
[328-188]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oae5981/
[328-199]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaf49c4/
[328-230]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oafuaqz/
[328-235]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oajrnk6/
[328-271]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaeq46w/
[328-296]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaegne7/
[328-304]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaf22jg/
[328-322]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaeuckp/
[328-426]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oajpqaz/
[328-463]: https://old.reddit.com/r/PathOfExileBuilds/comments/1rtfhfp/ama_crafting_ama_for_328_mirage_everything_you/oaly7i3/
[327-43]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnqz0ti/
[327-91]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnrnq2q/
[327-152]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnrhvlx/
[327-188]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsfkqg/
[327-191]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsez4n/
[327-194]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsej50/
[327-203]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnsbrm6/
[327-297]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxsv6n/
[327-330]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnyvk5l/
[327-306]: https://www.reddit.com/r/PathOfExileBuilds/comments/1orkytw/comment/nnxr9qu/
[326-26]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myz7s0r/
[326-46]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzpiul/
[326-49]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myz6him/
[326-82]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/myzd2p1/
[326-107]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz0kiap/
[326-287]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz4g4r2/
[326-210]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz1o6ha/
[326-344]: https://www.reddit.com/r/PathOfExileBuilds/comments/1lgu3b7/comment/mz77s6s/
[329-77]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p0yw4b1/
[329-135]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p11ak09/
[329-138]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p1elkwn/
[329-298]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p12dtli/
[329-304]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p12ezt2/
[329-301]: https://www.reddit.com/r/PathOfExileBuilds/comments/1vc1e72/comment/p12e65i/
