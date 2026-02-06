# Overview

This document summarizes the game's goals, scope, and design direction.

## Project Status

Beyond vs Below v0.1.0 | Status: Active | Last Updated: 2026-02-06

Quick Links: [README](../readme.md) | [Notes](./notes.md) | [Architecture](./architecture.md) | [Methodology](./methodology.md) | [TODO](./todo.md) | [Contributing](../contributing.md) | [Documentation Standards](../documentation_standards.md)

## High Concept

Beyond vs Below is a deterministic, turn-based 1v1 PvP dueling game with a third NPC faction (Humans) that escalates over time and can win the match. The core mode is capture-the-flag with a single win condition: return the flag to base before the Humans escape with it.

## Factions

### Aliens (Beyond)
- Precision technology, energy weapons, shields, psionics.
- Strong control and setup tools (pulls, positioning).
- Fragile but efficient.

### Demons (Below)
- Brutes, corruption, summoning, raw force.
- Strong pushes and durability.
- Slower, chaotic, punishing at close range.

### Humans (NPC Faction)
- Satirical modern-day archetypes.
- Operate as an escalating third faction.
- Can capture the flag and escape with it to win.
- Primary role: destabilize stalemates and punish the leader.

## Core Mode: Capture the Flag

- One flag, one return = win.
- No score, no rounds.
- Whoever holds the flag becomes the hunted.
- The environment grows more hostile the longer the flag is in play.

### Victory Conditions

- Aliens / Demons: return the flag to base.
- Humans: escape the map with the flag.
- Only one victory per match.

## The Living Chaos Engine (Humans)

### Escalating Event Waves

- Waves trigger on a fixed timer.
- Composition is semi-randomized.
- Difficulty escalates by tier.
- Waves bias toward attacking the leading player.

### Human Archetypes

- Boomers: slow shamblers, attrition pressure.
- Gym Bros: tanky bruisers, knockback, lane disruption.
- Karens: swarmers, scream debuffs, overwhelm by numbers.
- Bosses: Mega Karen, Bro Titan, Boomer Horde.

### Human Flag Behavior

- Humans can pick up the flag once escalation begins.
- Flag carrier forms an escort wave.
- Move toward map-specific escape routes.
- Slower than players, heavily telegraphed.
- Both players may temporarily cooperate to stop them.

## Core Mechanics

### Push-First Combat (Minimal CC)

Push is the primary form of crowd control.

- Push: 1-cell knockback, damage scales with proximity (closer = more damage).
- Pull: no damage, setup tool for later push.
- Charged Push: spend secondary action, next turn pushes 2 cells.
- Interactions:
  - Push into hazards (lava, turrets, collapse tiles).
  - Push into NPCs to force aggro.
  - Gym Bros can push players back.

### Environmental Synergy

- Hazards, NPCs, and terrain are weapons.
- Positioning > raw damage.
- Wave pressure replaces traditional hard CC.

### Turn Timer & Defaults

- If a player times out, units perform a default action (move, push, attack, or defend).
- Prevents dead turns and keeps momentum.

## Map Design

### Shared Layout Concepts

- Three lanes, vertical orientation.
- Side rooms for grinding mobs and buffs.
- Central lane is fastest but most dangerous.

### Maps

1. Human Level
   - Evacuation corridors.
   - Boomers, Karens, Bros, and the Titan boss.

2. Demon Lair
   - Molten rivers, bone altars.
   - Prisoners mutate into mobs.
   - Collapse = human escape.

3. Alien Ship
   - Sterile corridors, gravity shifts.
   - Captives hack terminals.
   - Escape pods = human win.

## Match Flow

1. Early: positioning, side-room buffs, light skirmishes.
2. Mid: flag contested, first escalation waves.
3. Late: boss waves, heavy hazards, chaos dominance.
4. End: flag returned or humans escape; match always climaxes.

## Aesthetic Direction

### Visual Style

- 16-bit era pixel aesthetic.
- SNES / Genesis inspired palettes.
- Chunky pixels, clean silhouettes, expressive faces.
- Satirical tone, exaggerated proportions.

### Tone

- Darkly comic.
- Aliens = cold order.
- Demons = raw destruction.
- Humans = irrational survival chaos.
