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

use std::cmp::max;
use std::collections::HashMap;
use std::{
    fmt,
    io::{Error, ErrorKind},
    str::FromStr,
};

pub mod armor;
pub use self::armor::Armor;

pub mod attack;
pub use self::attack::AccuracyKind;
pub use self::attack::Attack;
pub use self::attack::AttackKind;

pub mod attribute;
pub use self::attribute::Attribute;
pub use self::attribute::AttributeList;

pub mod bonus;
pub use self::bonus::AttackBonuses;
pub use self::bonus::Bonus;
pub use self::bonus::BonusKind;
pub use self::bonus::BonusList;

pub mod damage;
pub use self::damage::Damage;
pub use self::damage::DamageKind;
pub use self::damage::DamageList;

pub mod resistance;
pub use self::resistance::Resistance;

pub mod stat_list;
pub use self::stat_list::StatList;

use crate::area::LocationKind;
use sulis_core::ui::{color, Color};
use sulis_core::util::{gen_rand, invalid_data_error};

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    pub id: String,
    pub base_accuracy: u32,
    pub base_defense: u32,
    pub base_ap: u32,
    pub max_overflow_ap: i32,
    pub min_overflow_ap: i32,
    pub max_ap: u32,
    pub movement_ap: u32,
    pub attack_ap: u32,
    pub display_ap: u32,
    pub swap_weapons_ap: u32,
    pub initiative_roll_max: i32,
    pub base_flanking_angle: i32,
    pub graze_percentile: u32,
    pub hit_percentile: u32,
    pub crit_chance: u32,

    pub flanking_accuracy_bonus: i32,
    pub hidden_accuracy_bonus: i32,

    pub graze_damage_multiplier: f32,
    pub crit_damage_multiplier: f32,

    pub dual_wield_damage_multiplier: f32,

    pub base_attribute: i32,
    pub builder_max_attribute: i32,
    pub builder_min_attribute: i32,
    pub builder_attribute_points: i32,

    pub max_dialog_distance: f32,
    pub max_transition_distance: f32,
    pub max_prop_distance: f32,

    pub selectable_races: Vec<String>,
    pub selectable_classes: Vec<String>,
    pub ability_groups: Vec<String>,

    pub experience_factor: f32,
    pub experience_for_level: Vec<u32>,

    pub combat_run_away_vis_factor: f32,
    pub loot_drop_prop: String,

    pub item_weight_display_factor: f32,
    pub item_value_display_factor: f32,

    pub coins_item: String,
    pub unidentified_item_adjective: String,

    armor_damage_reduction_cap: Vec<u32>,

    pub rounds_per_hour: u32,
    pub hours_per_day: u32,
    pub hour_names: Vec<String>,

    pub area_colors: HashMap<LocationKind, Vec<Color>>,

    pub hints: Vec<String>,

    pub main_menu_music: Option<String>,
}

impl Rules {
    pub fn play_main_menu_music(&self) {
        if let Some(music) = self.main_menu_music.as_ref() {
            sulis_core::io::Audio::play_music(music, 1.0);
        }
    }

    pub fn random_hint(&self) -> String {
        match self.hints.len() {
            0 => "",
            1 => &self.hints[0],
            _ => {
                let index = gen_rand(0, self.hints.len() - 1);
                &self.hints[index]
            }
        }
        .to_string()
    }

    pub fn to_display_ap(&self, ap: i32) -> i32 {
        ap / self.display_ap as i32
    }

    pub fn format_ap(&self, ap: i32) -> String {
        let amount = (ap as f32 * 10.0 / self.display_ap as f32).floor();
        format!("{:.1}", amount / 10.0)
    }

    pub fn canonicalize_time(&self, time: &mut Time) {
        let mut millis = time.millis;
        let mut round = time.round;
        let mut hour = time.hour;
        let mut day = time.day;

        if millis >= ROUND_TIME_MILLIS {
            round += millis / ROUND_TIME_MILLIS;
            millis %= ROUND_TIME_MILLIS;
        }

        if round >= self.rounds_per_hour {
            hour += round / self.rounds_per_hour;
            round %= self.rounds_per_hour;
        }

        if hour >= self.hours_per_day {
            day += hour / self.hours_per_day;
            hour %= self.hours_per_day;
        }

        time.millis = millis;
        time.round = round;
        time.hour = hour;
        time.day = day;
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.hour_names.len() != self.hours_per_day as usize {
            return invalid_data_error(&format!(
                "Must specify '{}' hours names to match number of hours",
                self.hours_per_day
            ));
        }

        for (_, colors) in self.area_colors.iter() {
            if colors.len() != self.hours_per_day as usize {
                return invalid_data_error(&format!(
                    "Must specify '{}' hours for each area_colors.",
                    self.hours_per_day
                ));
            }
        }

        Ok(())
    }

    pub fn compute_millis(&self, time: Time) -> usize {
        let mut millis = time.millis as usize;

        let mut factor = ROUND_TIME_MILLIS as usize;
        millis += time.round as usize * factor;

        factor *= self.rounds_per_hour as usize;
        millis += time.hour as usize * factor;

        factor *= self.hours_per_day as usize;
        millis += time.day as usize * factor;

        millis
    }

    pub fn get_hour_name(&self, hour: u32) -> &str {
        assert!((hour as usize) < self.hour_names.len());

        &self.hour_names[hour as usize]
    }

    pub fn get_area_color(&self, location_kind: LocationKind, time: Time) -> Color {
        let (prev, next) = match self.area_colors.get(&location_kind) {
            None => return color::WHITE,
            Some(colors) => {
                let prev = time.hour as usize;
                if prev == colors.len() - 1 {
                    (colors[prev], colors[0])
                } else {
                    (colors[prev], colors[prev + 1])
                }
            }
        };

        let round_frac = 1.0 / self.rounds_per_hour as f32;
        let next_frac =
            round_frac * (time.round as f32 + time.millis as f32 / ROUND_TIME_MILLIS as f32);
        let prev_frac = 1.0 - next_frac;

        let r = prev_frac * prev.r + next_frac * next.r;
        let g = prev_frac * prev.g + next_frac * next.g;
        let b = prev_frac * prev.b + next_frac * next.b;
        let a = prev_frac * prev.a + next_frac * next.a;
        Color { r, g, b, a }
    }

    /// Computes the amount of damage that this damage list will apply to the given
    /// `armor`.  Each damage component of this list is rolled randomly, with the resulting
    /// damage then multiplied by the `multiplier`, rounded down.  The damage is then
    /// modified by the percentage resistance, if any.  The armor against
    /// the base damage kind of this damage is then subtracted from the damage, capped
    /// by the armor damage reduction cap for that armor value.  The
    /// resulting vector may be an empty vector to indicate no damage, or a vector of
    /// one or more kinds each associated with a positive damage amount.  The damage
    /// amount for each entry will never be zero.
    pub fn roll_damage(
        &self,
        damage: &DamageList,
        armor: &Armor,
        resistance: &Resistance,
        multiplier: f32,
    ) -> Vec<(DamageKind, u32)> {
        debug!(
            "Rolling damage from {} to {} vs {} base armor",
            damage.min(),
            damage.max(),
            armor.base()
        );

        if damage.is_empty() {
            return Vec::new();
        }

        let mut output = Vec::new();
        for damage in damage.iter() {
            let kind = damage.kind.unwrap();

            let resistance = (100 - resistance.amount(kind)) as f32 / 100.0;
            let amount = damage.roll() as f32 * multiplier * resistance;

            let armor = max(0, armor.amount(kind) as i32 - damage.ap as i32) as u32;
            let armor_max = self.armor_damage_reduction_cap(armor) as f32 * amount / 100.0;
            let armor = armor as f32;

            let armor = if armor_max > armor { armor } else { armor_max };
            let armor = if armor > amount { amount } else { armor };

            let amount = amount - armor;
            if amount > 0.0 {
                output.push((kind, amount.ceil() as u32));
            }
        }

        output
    }

    /// Returns the percentile armor reduction cap for the given armor value.  this
    /// is the maximum percentage that the armor of that level can reduce a damage
    /// amount by.  the remaining damage is rounded up.
    pub fn armor_damage_reduction_cap(&self, armor: u32) -> u32 {
        *self
            .armor_damage_reduction_cap
            .get(armor as usize)
            .unwrap_or(&100)
    }

    pub fn get_xp_for_next_level(&self, cur_level: u32) -> u32 {
        if cur_level < 1 {
            return 0;
        }
        if cur_level > self.experience_for_level.len() as u32 {
            return 0;
        }

        self.experience_for_level[(cur_level - 1) as usize]
    }

    pub fn concealment_roll(&self, concealment: i32) -> bool {
        if concealment == 0 {
            return true;
        }
        let roll = gen_rand(1, 101);
        debug!("Concealment roll: {} against {}", roll, concealment);
        roll > concealment
    }
}

pub const ROUND_TIME_MILLIS: u32 = 5000;

#[derive(Deserialize, Serialize, Debug, Copy, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Time {
    #[serde(default)]
    pub day: u32,

    #[serde(default)]
    pub hour: u32,

    #[serde(default)]
    pub round: u32,

    #[serde(default)]
    pub millis: u32,
}

impl Time {
    /// Creates a time with the specified number of hours.
    /// Does not canonicalize the time.
    pub fn from_hours(hours: u32) -> Time {
        Time {
            day: 0,
            hour: hours,
            round: 0,
            millis: 0,
        }
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut prev = false;
        if self.day > 0 {
            write_singular_or_plural(f, prev, self.day, "day")?;
            prev = true;
        }

        if self.hour > 0 {
            write_singular_or_plural(f, prev, self.hour, "hour")?;
            prev = true;
        }

        if self.round > 0 {
            write_singular_or_plural(f, prev, self.round, "round")?;
            prev = true;
        }

        if self.millis > 0 {
            write_singular_or_plural(f, prev, self.millis, "milli")?;
        }

        Ok(())
    }
}

fn write_singular_or_plural(
    f: &mut fmt::Formatter,
    prev: bool,
    qty: u32,
    unit: &str,
) -> fmt::Result {
    if prev {
        write!(f, ", ")?;
    }

    write!(f, "{} {}", qty, unit)?;

    if qty > 1 {
        write!(f, "s")?;
    }

    Ok(())
}

impl Default for Time {
    fn default() -> Self {
        Time {
            day: 0,
            hour: 0,
            round: 0,
            millis: 0,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub enum Slot {
    Cloak,
    Head,
    Torso,
    Hands,
    HeldMain,
    HeldOff,
    Legs,
    Feet,
    Waist,
    Neck,
    FingerMain,
    FingerOff,
}

impl Slot {
    pub fn iter() -> ::std::slice::Iter<'static, Slot> {
        SLOTS_LIST.iter()
    }
}

impl FromStr for Slot {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = match s {
            "cloak" => Slot::Cloak,
            "head" => Slot::Head,
            "torso" => Slot::Torso,
            "hands" => Slot::Hands,
            "held_main" => Slot::HeldMain,
            "held_off" => Slot::HeldOff,
            "legs" => Slot::Legs,
            "feet" => Slot::Feet,
            "waist" => Slot::Waist,
            "neck" => Slot::Neck,
            "finger_main" => Slot::FingerMain,
            "finger_off" => Slot::FingerOff,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unable to parse Slot from '{}'", s),
                ));
            }
        };

        Ok(val)
    }
}

use self::Slot::*;

// The sort order of this list is important
const SLOTS_LIST: [Slot; 12] = [
    Cloak, Feet, Legs, Torso, Hands, Head, HeldMain, HeldOff, Waist, Neck, FingerMain, FingerOff,
];

#[derive(Deserialize, Serialize, Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub enum QuickSlot {
    AltHeldMain,
    AltHeldOff,
    Usable1,
    Usable2,
    Usable3,
    Usable4,
}

impl QuickSlot {
    pub fn iter() -> ::std::slice::Iter<'static, QuickSlot> {
        QUICKSLOTS_LIST.iter()
    }

    pub fn usable_iter() -> ::std::slice::Iter<'static, QuickSlot> {
        USABLE_QUICKSLOTS_LIST.iter()
    }
}

use self::QuickSlot::*;

const QUICKSLOTS_LIST: [QuickSlot; 6] =
    [AltHeldMain, AltHeldOff, Usable1, Usable2, Usable3, Usable4];

const USABLE_QUICKSLOTS_LIST: [QuickSlot; 4] = [Usable1, Usable2, Usable3, Usable4];

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ItemKind {
    Armor { kind: ArmorKind },
    Weapon { kind: WeaponKind },
    Other,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub struct HitFlags {
    pub flanking: bool,
    pub sneak_attack: bool,
    pub concealment: bool,
}

impl Default for HitFlags {
    fn default() -> Self {
        HitFlags {
            flanking: false,
            sneak_attack: false,
            concealment: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord)]
pub enum HitKind {
    Miss,
    Graze,
    Hit,
    Crit,
    Auto,
}

impl FromStr for HitKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = match s {
            "miss" => HitKind::Miss,
            "graze" => HitKind::Graze,
            "hit" => HitKind::Hit,
            "crit" => HitKind::Crit,
            "auto" => HitKind::Auto,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unable to parse HitKind from '{}'", s),
                ));
            }
        };

        Ok(val)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum WeaponStyle {
    Ranged,
    TwoHanded,
    Single,
    Shielded,
    DualWielding,
}

impl FromStr for WeaponStyle {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = match s {
            "ranged" => WeaponStyle::Ranged,
            "two_handed" => WeaponStyle::TwoHanded,
            "single" => WeaponStyle::Single,
            "shielded" => WeaponStyle::Shielded,
            "dual_wielding" => WeaponStyle::DualWielding,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unable to parse WeaponStyle from '{}'", s),
                ));
            }
        };

        Ok(val)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum WeaponKind {
    Axe,
    Crossbow,
    Bow,
    SmallSword,
    LargeSword,
    Hammer,
    Spear,
    Mace,
    Simple,
}

impl FromStr for WeaponKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = match s {
            "axe" => WeaponKind::Axe,
            "crossbow" => WeaponKind::Crossbow,
            "bow" => WeaponKind::Bow,
            "small_sword" => WeaponKind::SmallSword,
            "large_sword" => WeaponKind::LargeSword,
            "hammer" => WeaponKind::Hammer,
            "spear" => WeaponKind::Spear,
            "mace" => WeaponKind::Mace,
            "simple" => WeaponKind::Simple,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unable to parse WeaponKind from '{}'", s),
                ));
            }
        };

        Ok(val)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum ArmorKind {
    Light,
    Medium,
    Heavy,
}

impl FromStr for ArmorKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let val = match s {
            "light" => ArmorKind::Light,
            "medium" => ArmorKind::Medium,
            "heavy" => ArmorKind::Heavy,
            _ => {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("Unable to parse ArmorKind from '{}'", s),
                ));
            }
        };

        Ok(val)
    }
}
