-- builds the scene once (the first tick), then steps the solver: 16 substeps of one pass each
local N = 9

-- a point: x y old_x old_y inv_mass radius layer mask (w = 0 is fixed)
local function point(name, x, y, w, r, layer, mask)
  ecs.spawn(name, {x = x, y = y, old_x = x, old_y = y, inv_mass = w, radius = r,
    layer = layer, mask = mask, contact = 0, contact_nx = 0, contact_ny = 0})
end

-- a link between two named points; road links also have a layer (they carry the ball)
local function link(name, a, b, rest, comp, mode, layer)
  local c = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = comp,
    mode = mode, lambda = 0, force_max = 0}
  if layer then c.layer = layer c.friction = 0.8 end
  ecs.spawn(name, c)
end

local function build()
  point('anchor_l', 100, 300, 0, 0, 1, 1)
  point('anchor_r', 500, 300, 0, 0, 1, 1)
  local prev = 'anchor_l'
  for i = 1, N do
    local name = 'joint_' .. i
    point(name, 100 + i * 40, 300, 1, 3, 1, 1)
    link('beam_' .. i, prev, name, 40, 0, 0, 4)
    prev = name
  end
  link('beam_' .. (N + 1), prev, 'anchor_r', 40, 0, 0, 4)
  point('ball', 300, 200, 1, 12, 2, 7)
  -- a cable above: two anchors, a link that only holds the length, and a catenary to draw
  point('cable_l', 100, 100, 0, 0, 1, 1)
  point('cable_r', 500, 100, 0, 0, 1, 1)
  link('cable', 'cable_l', 'cable_r', 450, 0, 1, nil)
  ecs.set('cable', 'catenary_a', 0)
  ecs.set('cable', 'catenary_x0', 0)
  ecs.set('cable', 'catenary_y0', 0)
  ecs.set('cable', 'sag', 0)
  ecs.set('cable', 'linear_density', 0.05)
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
  ecs.fill('force_max', 0)
  for i = 1, 16 do substep(dt / 16) end
  ecs.update_catenary(980)
end
