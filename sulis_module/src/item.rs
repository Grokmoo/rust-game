//  This file is part of Sulis, a turn based RPG written in Rust.
//  Copyright 2018 Jared Stephen
//
//  Sulis is free software: you can redistribute it and/or modify
//  it under the terms of the GNU General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  Sulis is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU General Public License for more details.
//
//  You should have received a copy of the GNU General Public License
//  along with Sulis.  If not, see <http://www.gnu.org/licenses/>

use std::cmp;
use std::collections::hash_map::Iter;
use std::collections::HashMap;
use std::io::Error;
use std::rc::Rc;

use crate::rules::{bonus::AttackBuilder, BonusList, ItemKind, Slot};
use sulis_core::image::Image;
use sulis_core::resource::ResourceSet;
use sulis_core::util::unable_to_create_error;

use crate::{
    ability::{AIData, Duration},
    Actor, ImageLayer, ItemAdjective, Module, PrereqList, PrereqListBuilder,
};

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Equippable {
    pub slot: Slot,
    pub alternate_slot: Option<Slot>,
    pub blocks_slot: Option<Slot>,
    pub bonuses: BonusList,
    pub attack: Option<AttackBuilder>,
}

#[derive(Debug, Clone)]
pub struct Usable {
    pub script: String,
    pub ap: u32,
    pub duration: Duration,
    pub consumable: bool,
    pub short_description: String,
    pub ai: AIData,
    pub use_in_slot: bool,
}

#[derive(Debug, Clone)]
struct Variant {
    image: HashMap<ImageLayer, Rc<dyn Image>>,
    alternate_image: HashMap<ImageLayer, Rc<dyn Image>>,
    icon: Rc<dyn Image>,
}

#[derive(Debug)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub unidentified_name: Option<String>,
    pub kind: ItemKind,
    pub equippable: Option<Equippable>,
    pub prereqs: Option<PrereqList>,
    pub value: i32,
    pub weight: i32,
    pub usable: Option<Usable>,

    // original values from before any adjectives are applied
    pub original_id: String,
    original_value: i32,
    original_equippable: Option<Equippable>,

    // adjectives that were present in the item builder when this
    // item is defined
    builder_adjectives: Vec<Rc<ItemAdjective>>,

    // adjectives that were not in the item builder but were created
    // dynamically
    pub added_adjectives: Vec<Rc<ItemAdjective>>,

    icon: Rc<dyn Image>,
    image: HashMap<ImageLayer, Rc<dyn Image>>,
    alternate_image: HashMap<ImageLayer, Rc<dyn Image>>,
    variants: Vec<Variant>,
}

fn build_hash_map(
    id: &str,
    input: HashMap<ImageLayer, String>,
) -> Result<HashMap<ImageLayer, Rc<dyn Image>>, Error> {
    let mut output = HashMap::new();
    for (layer, image_id) in input {
        let image = read_image(&image_id, id)?;
        output.insert(layer, image);
    }

    Ok(output)
}

fn read_image(image_id: &str, id: &str) -> Result<Rc<dyn Image>, Error> {
    match ResourceSet::image(image_id) {
        None => {
            warn!("No image found for image '{}'", image_id);
            unable_to_create_error("item", id)
        }
        Some(image) => Ok(image),
    }
}

impl Item {
    pub fn clone_with_adjectives(
        item: &Rc<Item>,
        mut adjectives: Vec<Rc<ItemAdjective>>,
        new_id: String,
    ) -> Item {
        assert!(!adjectives.is_empty());

        let mut added_adjectives = item.added_adjectives.clone();
        added_adjectives.append(&mut adjectives);

        let mut all_adjectives = item.builder_adjectives.clone();
        all_adjectives.extend_from_slice(&added_adjectives);

        let (equippable, value, prereqs) = apply_adjectives(
            &item.prereqs,
            &item.original_equippable,
            item.original_value,
            &all_adjectives,
        );

        let mut unidentified_name = None;

        let mut name = String::new();
        for adj in all_adjectives.iter() {
            if let Some(ref prefix) = adj.name_prefix {
                if unidentified_name.is_none() &&
                    "Exceptional Fine Masterwork Rusty Worn ".find(prefix).is_none() {
                    unidentified_name = Some("Unidentified ".to_string() + &item.name);
                }
                name.push_str(prefix);
            }
        }
        name.push_str(&item.name);
        for adj in all_adjectives.iter() {
            if let Some(ref postfix) = adj.name_postfix {
                if unidentified_name.is_none() {
                    unidentified_name = Some("Unidentified ".to_string() + &item.name);
                }
                name.push_str(postfix);
            }
        }

        Item {
            id: new_id,
            icon: Rc::clone(&item.icon),
            image: item.image.clone(),
            kind: item.kind,
            alternate_image: item.alternate_image.clone(),
            name,
            unidentified_name,
            equippable,
            value,
            weight: item.weight,
            usable: item.usable.clone(),
            prereqs,
            original_id: item.original_id.clone(),
            original_value: item.original_value,
            original_equippable: item.original_equippable.clone(),
            builder_adjectives: item.builder_adjectives.clone(),
            added_adjectives,
            variants: item.variants.clone(),
        }
    }

    pub fn new(builder: ItemBuilder, module: &Module) -> Result<Item, Error> {
        if let Some(equippable) = &builder.equippable {
            if let Some(attack) = &equippable.attack {
                if attack.damage.kind.is_none() {
                    warn!("Kind must be specified for attack damage.");
                    return unable_to_create_error("item", &builder.id);
                }
            }
        }

        let usable = match builder.usable {
            None => None,
            Some(usable) => {
                match module.scripts.get(&usable.script) {
                    None => {
                        warn!("No script found with id '{}'", usable.script);
                        return unable_to_create_error("item", &builder.id);
                    }
                    Some(_) => (),
                };

                Some(Usable {
                    script: usable.script,
                    consumable: usable.consumable,
                    duration: usable.duration,
                    ap: usable.ap,
                    short_description: usable.short_description,
                    ai: usable.ai,
                    use_in_slot: usable.use_in_slot,
                })
            }
        };

        let prereqs = match builder.prereqs {
            None => None,
            Some(list) => Some(PrereqList::new(list)?),
        };

        let mut adjectives = Vec::new();
        for adj_id in builder.adjectives {
            let adjective = match module.item_adjectives.get(&adj_id) {
                None => {
                    warn!("No item adjective found with id '{}'", adj_id);
                    return unable_to_create_error("item", &builder.id);
                }
                Some(ref adj) => Rc::clone(adj),
            };
            adjectives.push(adjective);
        }

        let icon = read_image(&builder.icon, &builder.id)?;
        let images = build_hash_map(&builder.id, builder.image)?;
        let alt_images = build_hash_map(&builder.id, builder.alternate_image)?;

        let mut variants = Vec::new();
        for variant in builder.variants {
            let icon = read_image(&variant.icon, &builder.id)?;
            let image = build_hash_map(&builder.id, variant.image)?;
            let alternate_image = build_hash_map(&builder.id, variant.alternate_image)?;
            variants.push(Variant {
                icon,
                image,
                alternate_image,
            });
        }

        let (equippable, value, prereqs) = apply_adjectives(
            &prereqs,
            &builder.equippable,
            builder.value as i32,
            &adjectives,
        );

        Ok(Item {
            id: builder.id.clone(),
            icon,
            image: images,
            kind: builder.kind.unwrap_or(ItemKind::Other),
            alternate_image: alt_images,
            name: builder.name,
            unidentified_name: builder.unidentified_name,
            equippable,
            value,
            weight: builder.weight as i32,
            usable,
            prereqs,
            original_id: builder.id,
            original_value: builder.value as i32,
            original_equippable: builder.equippable,
            builder_adjectives: adjectives,
            added_adjectives: Vec::new(),
            variants,
        })
    }

    pub fn num_variants(&self) -> usize {
        self.variants.len()
    }

    pub fn adjective_icons(&self) -> Vec<String> {
        self.builder_adjectives
            .iter()
            .chain(self.added_adjectives.iter())
            .map(|adj| adj.item_status_icon.id())
            .collect()
    }

    pub fn meets_prereqs(&self, actor: &Rc<Actor>) -> bool {
        match self.prereqs {
            None => true,
            Some(ref prereqs) => prereqs.meets(actor),
        }
    }

    pub fn icon(&self, variant: Option<usize>) -> Rc<dyn Image> {
        Rc::clone(match variant {
            None => &self.icon,
            Some(idx) => &self.variants[idx].icon,
        })
    }

    pub fn alt_image_iter(&self, variant: Option<usize>) -> Iter<ImageLayer, Rc<dyn Image>> {
        match variant {
            None => self.alternate_image.iter(),
            Some(idx) => self.variants[idx].alternate_image.iter(),
        }
    }

    pub fn image_iter(&self, variant: Option<usize>) -> Iter<ImageLayer, Rc<dyn Image>> {
        match variant {
            None => self.image.iter(),
            Some(idx) => self.variants[idx].image.iter(),
        }
    }

    pub fn is_armor(&self) -> bool {
        match self.kind {
            ItemKind::Armor { .. } => true,
            _ => false,
        }
    }

    pub fn is_weapon(&self) -> bool {
        match self.kind {
            ItemKind::Weapon { .. } => true,
            _ => false,
        }
    }
}

fn apply_adjectives(
    base_prereqs: &Option<PrereqList>,
    equippable: &Option<Equippable>,
    value: i32,
    adjectives: &[Rc<ItemAdjective>],
) -> (Option<Equippable>, i32, Option<PrereqList>) {
    // first, add bonuses from adjectives and accumulate modifiers
    let mut value_add = 0;
    let mut value_modifier = 1.0;
    let mut bonus_modifier = 1.0;
    let mut penalty_modifier = 1.0;
    let mut attack_damage_mod = 1.0;
    let mut attack_bonus_mod = 1.0;
    let mut attack_penalty_mod = 1.0;

    let mut prereqs = base_prereqs.clone();
    let mut equippable = equippable.clone();
    for adjective in adjectives.iter() {
        value_add += adjective.value_add.unwrap_or(0);
        value_modifier += adjective.value_modifier.unwrap_or(1.0) - 1.0;
        bonus_modifier += adjective.bonus_modifier.unwrap_or(1.0) - 1.0;
        penalty_modifier += adjective.penalty_modifier.unwrap_or(1.0) - 1.0;
        attack_damage_mod += adjective.attack_damage_modifier.unwrap_or(1.0) - 1.0;
        attack_bonus_mod += adjective.attack_bonus_modifier.unwrap_or(1.0) - 1.0;
        attack_penalty_mod += adjective.attack_penalty_modifier.unwrap_or(1.0) - 1.0;

        if let Some(ref mut equippable) = equippable {
            for bonus in adjective.bonuses.iter() {
                equippable.bonuses.add(bonus.clone());
            }

            if let Some(ref mut attack) = equippable.attack {
                attack.bonuses.add(&adjective.attack_bonuses);
            }
        }

        prereqs = PrereqList::merge_optional(&prereqs, &adjective.prereqs);
    }

    // now apply accumulated bonuses and penalties
    if let Some(ref mut equippable) = equippable {
        equippable.bonuses.merge_duplicates();

        if let Some(ref mut attack) = equippable.attack {
            if (attack_bonus_mod - 1.0).abs() > std::f32::EPSILON
                || (attack_penalty_mod - 1.0).abs() > std::f32::EPSILON
            {
                attack
                    .bonuses
                    .apply_modifier(attack_penalty_mod, attack_bonus_mod);
            }

            if (attack_damage_mod - 1.0).abs() > std::f32::EPSILON {
                let damage = attack.damage.mult_f32(attack_damage_mod);
                attack.damage = damage;
            }
        }

        equippable
            .bonuses
            .apply_modifiers(penalty_modifier, bonus_modifier);
    }

    let value = cmp::max(
        0,
        (value as f32 * value_modifier).round() as i32 + value_add,
    );

    (equippable, value, prereqs)
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct UsableBuilder {
    pub script: String,
    pub ap: u32,
    pub duration: Duration,
    pub consumable: bool,
    pub short_description: String,
    pub ai: AIData,
    #[serde(default = "bool_true")]
    pub use_in_slot: bool,
}

fn bool_true() -> bool {
    true
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct VariantBuilder {
    #[serde(default)]
    image: HashMap<ImageLayer, String>,

    #[serde(default)]
    alternate_image: HashMap<ImageLayer, String>,

    #[serde(default)]
    icon: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ItemBuilder {
    pub id: String,
    pub name: String,
    pub unidentified_name: Option<String>,
    kind: Option<ItemKind>,
    pub icon: String,
    pub equippable: Option<Equippable>,

    #[serde(default)]
    pub image: HashMap<ImageLayer, String>,

    #[serde(default)]
    pub alternate_image: HashMap<ImageLayer, String>,

    prereqs: Option<PrereqListBuilder>,
    value: u32,
    weight: u32,
    usable: Option<UsableBuilder>,
    #[serde(default)]
    adjectives: Vec<String>,

    #[serde(default)]
    variants: Vec<VariantBuilder>,
}

pub fn format_item_value(value: i32) -> String {
    let display_factor = Module::rules().item_value_display_factor;

    let value_f = value as f32 / display_factor;
    format!("{:.*}", 1, value_f)
}

pub fn format_item_weight(weight: i32) -> String {
    let display_factor = Module::rules().item_weight_display_factor;

    let weight_f = weight as f32 / display_factor;
    format!("{:.*}", 2, weight_f)
}
