SONG_ID = "song_of_heroes"
SONG_NAME = "Song of Heroes"

function on_activate(parent, ability)
  if not parent:has_effect_with_tag("singing_" .. SONG_ID) then
	game:say_line("This verse must be sung with the " .. SONG_NAME)
	return
  end

  local radius = ability:range() + parent:ability_level_from_id("louder_music") * 2

  local targets = parent:targets():friendly()
  local targeter = parent:create_targeter(ability)
  targeter:add_selectable(parent)
  targeter:set_selection_radius(radius)
  targeter:set_shape_circle(radius)
  targeter:add_all_effectable(targets)
  targeter:invis_blocks_affected_points(true)
  targeter:activate()
end

function on_target_select(parent, ability, targets)
  ability:activate(parent)
  
  local gen = parent:create_particle_generator("note", 1.0)
  gen:set_initial_gen(20.0)
  gen:set_position(gen:param(parent:center_x()), gen:param(parent:center_y() - 0.5))
  gen:set_gen_rate(gen:param(0.0))
  gen:set_particle_size_dist(gen:fixed_dist(0.7), gen:fixed_dist(0.7))
  gen:set_particle_position_dist(gen:dist_param(gen:uniform_dist(-0.4, 0.4), gen:uniform_dist(-2.0, 2.0)),
                                  gen:dist_param(gen:uniform_dist(-0.4, 0.4), gen:uniform_dist(-2.0, 2.0)))
  gen:set_particle_duration_dist(gen:fixed_dist(1.0))
  gen:set_alpha(gen:param(1.0, -1.0))
  gen:activate()
  
  local targets = targets:to_table()
  for i = 1, #targets do
    apply_effect(parent, ability, targets[i])
  end
end

function apply_effect(parent, ability, target)
  local stats = parent:stats()
  local amount = 2 + stats.caster_level / 4 + stats.perception_bonus / 4
  
  local effect = target:create_effect(ability:name(), ability:duration())
  effect:add_num_bonus("caster_level", 1 + stats.caster_level / 5)
  effect:add_damage(amount, amount * 2, 2, "attack_with_damage_kind Acid")
  effect:add_damage(amount, amount * 2, 2, "attack_with_damage_kind Cold")
  effect:add_damage(amount, amount * 2, 2, "attack_with_damage_kind Shock")
  effect:add_damage(amount, amount * 2, 2, "attack_with_damage_kind Fire")

  local anim = target:create_anim("spell_shock")
  anim:set_moves_with_parent()
  anim:set_position(anim:param(-0.5), anim:param(-2.5))
  anim:set_particle_size_dist(anim:fixed_dist(1.0), anim:fixed_dist(1.0))
  anim:set_color(anim:param(1.0), anim:param(0.0), anim:param(0.0), anim:param(1.0))
  effect:add_anim(anim)
  effect:apply()
end
