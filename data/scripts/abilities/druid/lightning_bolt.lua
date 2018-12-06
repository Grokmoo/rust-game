function on_activate(parent, ability)
  targets = parent:targets()
  
  targeter = parent:create_targeter(ability)
  targeter:set_free_select(15.0)
  targeter:set_shape_object_size("3by3")
  targeter:add_all_effectable(targets)
  targeter:activate()
end

function on_target_select(parent, ability, targets)
  position = targets:selected_point()

  anim = parent:create_anim("lightning_bolt", 0.99)
  anim:set_position(anim:param(position.x - 2.0), anim:param(position.y - 10.0))
  anim:set_particle_size_dist(anim:fixed_dist(5.0), anim:fixed_dist(12.0))
  anim:set_alpha(anim:param(1.0))
  
  targets = targets:to_table()
  for i = 1, #targets do
    attack_target(parent, ability, targets[i])
  end
  
  anim:activate()
  ability:activate(parent)
end

function attack_target(parent, ability, target)
  stats = parent:stats()
  min_dmg = 18 + stats.caster_level / 2 + stats.wisdom_bonus / 4
  max_dmg = 28 + stats.wisdom_bonus / 2 + stats.caster_level
  parent:special_attack(target, "Reflex", "Spell", min_dmg, max_dmg, 5, "Electrical")
end
