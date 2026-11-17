-- a suspension bridge: a cable between two fixed points with one joint in the middle, a road
-- of 10 links between two fixed points, ONE hanger from the cable joint down to the middle of
-- the road, and a wheel that follows the mouse (a spring to a fixed point that sits at the
-- mouse). 16 substeps of one pass each
local N = 9

-- a point: x y old_x old_y inv_mass radius layer mask (w = 0 is fixed: a rigid fixing).
-- damping is the share of speed a point keeps per substep (nil: the call's 0.999). joints keep
-- 0.996 (4 times the loss of 0.999), the wheel 0.984 (16 times)
-- inv_mass is 1 / mass: a joint 0.1 is a mass of 10, the wheel 0.02 is a mass of 50
-- layer is what the point is (8 a joint, 16 a wheel, 32 the mouse marker), mask is what it hits:
-- 1 ground, 2 semi-solid, 4 road, 16 wheels. a joint hits only the ground
local function point(name, x, y, w, r, layer, mask, damping)
  ecs.spawn(name, {x = x, y = y, old_x = x, old_y = y, inv_mass = w, radius = r,
    layer = layer, mask = mask, contact = 0, contact_nx = 0, contact_ny = 0, damping = damping})
end

-- a link between two named points. mode 0 keeps the length, 1 only stops it from getting
-- longer (a rope), comp is the softness (0 is hard). road links have a layer (the wheel rides on them)
local function link(name, a, b, rest, comp, mode, layer)
  local c = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = comp,
    mode = mode, lambda = 0, force_max = 0}
  if layer then c.layer = layer c.friction = 0.8 end
  ecs.spawn(name, c)
end

local function cable(name, a, b, rest)
  link(name, a, b, rest, 0, 1, nil)
  ecs.set(name, 'catenary_a', 0)
  ecs.set(name, 'catenary_x0', 0)
  ecs.set(name, 'catenary_y0', 0)
  ecs.set(name, 'sag', 0)
  ecs.set(name, 'linear_density', 0.5)
end

local function build()
  -- the road: fixed at both ends, 10 links of 40
  point('road_l', 100, 300, 0, 0, 1, 1)
  point('road_r', 500, 300, 0, 0, 1, 1)
  local prev = 'road_l'
  for i = 1, N do
    point('joint_' .. i, 100 + i * 40, 300, 0.1, 3, 8, 1, 0.996)
    link('beam_' .. i, prev, 'joint_' .. i, 40, 0, 0, 4)
    prev = 'joint_' .. i
  end
  link('beam_' .. (N + 1), prev, 'road_r', 40, 0, 0, 4)
  -- the cable: fixed at both ends, one free joint in the middle
  point('cable_l', 100, 100, 0, 0, 1, 1)
  point('cable_r', 500, 100, 0, 0, 1, 1)
  point('hang', 300, 190, 0.1, 3, 8, 1, 0.996)
  cable('cable_a', 'cable_l', 'hang', 220)
  cable('cable_b', 'hang', 'cable_r', 220)
  -- the only hanger: a rope (it sags when slack, it never pushes) from the cable joint to the
  -- middle of the road. a strut (mode 2) would be the one that gets compressed
  cable('hanger', 'hang', 'joint_5', 110)
  -- the wheel and the mouse: a soft spring of length 0 pulls the wheel to the mouse
  point('wheel', 200, 250, 0.02, 12, 16, 23, 0.984)
  point('mouse', 200, 250, 0, 0, 32, 0)
  link('drag', 'mouse', 'wheel', 0, 0.00002, 0, nil)
  ecs.set('app', 'ready', 1)
end

local function substep(h)
  ecs.integrate_verlet(h, 0, 980, 0.999)
  ecs.apply_catenary_load(h, 980)
  ecs.solve_distance(h, 1)
  ecs.project_edges(1)
  ecs.project_segments(1)
end

return function(dt)
  if ecs.get('app', 'ready') ~= 1 then build() end
  -- the mouse marker sits where the pointer is (before the first move the pointer reads 0, 0:
  -- then the marker stays where it was)
  local mx, my = input.mouse()
  if mx ~= 0 or my ~= 0 then
    ecs.set('mouse', 'x', mx) ecs.set('mouse', 'y', my)
    ecs.set('mouse', 'old_x', mx) ecs.set('mouse', 'old_y', my)
  end
  ecs.fill('force_max', 0)
  for i = 1, 16 do substep(dt / 16) end
  ecs.update_catenary(980)
end
