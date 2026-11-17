-- a suspension bridge: a cable between two fixed points with one joint in the middle, a road
-- of 10 links between two fixed points, ONE hanger from the cable joint down to the middle of
-- the road, and a wheel that follows the mouse (a spring to a fixed point that sits at the
-- mouse). 16 substeps of one pass each
local N = 9
-- the road is a chain: its links are ropes (mode 1), they hold a pull and carry no push, so the
-- road cannot act as an arch. 10 links of 40.5 are 5 longer than the 400 between the fixings
local BEAM = 40.5
-- every beam has a weight per length; half of it sits on each of its two joints. a joint has a
-- small mass of its own (mass0) besides. so the mass of a joint is always computed from what is
-- attached to it (remass), never typed in
local DENS = 1.0
local JOINT0 = 1.0
-- the pieces of a broken part end in a new small joint: radius, and how much speed it keeps
-- (a little more loss than the bridge joints, so the halves do not spin like helicopters)
local DEBRIS_R = BEAM / 4
local DEBRIS_DAMP = 0.97
-- the start of the scene is a shock (a straight, taut road and cable begin to hang, forces of
-- 10^7 and more in the first ticks), so nothing is judged before this many ticks
local WARMUP = 30
-- a free joint farther than this from the world is removed with its links
local WORLD = {x0 = -200, x1 = 1000, y1 = 900}

-- a point: x y old_x old_y inv_mass radius layer mask (w = 0 is fixed: a rigid fixing).
-- damping is the share of speed a point keeps per substep (nil: the call's 0.999). joints keep
-- 0.996 (4 times the loss of 0.999), the wheel 0.984 (16 times)
-- inv_mass is 1 / mass: a joint 0.1 is a mass of 10, the wheel 0.02 is a mass of 50
-- layer is what the point is (8 a joint, 16 a wheel, 32 the mouse marker), mask is what it hits:
-- 1 ground, 2 semi-solid, 4 road, 16 wheels. a joint hits only the ground
local function point(name, x, y, w, r, layer, mask, damping, mass0)
  ecs.spawn(name, {x = x, y = y, old_x = x, old_y = y, inv_mass = w, radius = r,
    layer = layer, mask = mask, contact = 0, contact_nx = 0, contact_ny = 0, damping = damping,
    mass0 = mass0})
end

-- a link between two named points. mode 0 keeps the length, 1 only stops it from getting
-- longer (a rope), comp is the softness (0 is hard). road links have a layer (the wheel rides on them)
-- strength is the load the part carries (in the same units as force_max and tension) before it
-- would break; nothing breaks yet, the drawing only colors the load against it
local function link(name, a, b, rest, comp, mode, layer, strength)
  local c = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = comp,
    mode = mode, lambda = 0, force_max = 0, strength = strength}
  -- a road link is a beam: it has a weight per length and a layer (the wheel rides on it)
  if layer then c.layer = layer c.friction = 0.8 c.linear_density = DENS end
  ecs.spawn(name, c)
end

local function cable(name, a, b, rest, strength)
  link(name, a, b, rest, 0, 1, nil, strength)
  ecs.set(name, 'catenary_a', 0)
  ecs.set(name, 'catenary_x0', 0)
  ecs.set(name, 'catenary_y0', 0)
  ecs.set(name, 'sag', 0)
  ecs.set(name, 'linear_density', 0.5)
  -- a loaded cable is never exactly taut: at least 0.1 % of the chord is slack (its tension
  -- stays finite and keeps pulling the ends into balance)
  ecs.set(name, 'min_slack', 0.001)
  ecs.set(name, 'tension', 0)
end

-- every beam has a strength of its own: 1.0 to 1.4 million, a different one each
local function beam_strength(i)
  return 1000000 + 100000 * ((i * 7) % 5)
end

-- the mass of every free joint = its own mass0 + half the weight (length * linear_density) of
-- each beam on it. cables are not beams here: the solver loads their ends with their weight
local function remass()
  local n, ids, m0 = ecs.query('mass0')
  local sum = {}
  for i = 1, n do sum[ids[i]] = m0[i] end
  local k, _, a, b, rest, dens = ecs.query('node_a', 'node_b', 'rest_len', 'linear_density', 'layer')
  for i = 1, k do
    local m = rest[i] * dens[i] / 2
    local ea, eb = math.tointeger(a[i]), math.tointeger(b[i])
    if sum[ea] then sum[ea] = sum[ea] + m end
    if sum[eb] then sum[eb] = sum[eb] + m end
  end
  for i = 1, n do ecs.set(ids[i], 'inv_mass', 1 / sum[ids[i]]) end
end

-- a part that was judged too weak last tick is replaced now by two halves. each half hangs on
-- an old joint and ends in a new small joint at the middle; the halves cannot break. a beam
-- leaves two rigid half beams (they weigh as beams, so remass moves a quarter of the beam to each
-- new joint). a rope leaves two half ropes that hang slack
local function break_part(id)
  local a, b = math.tointeger(ecs.get(id, 'node_a')), math.tointeger(ecs.get(id, 'node_b'))
  local xa, ya, xb, yb = ecs.get(a, 'x'), ecs.get(a, 'y'), ecs.get(b, 'x'), ecs.get(b, 'y')
  local mx, my = (xa + xb) / 2, (ya + yb) / 2
  -- the new joints start with the mean speed of the two old ends
  local vx = ((xa - ecs.get(a, 'old_x')) + (xb - ecs.get(b, 'old_x'))) / 2
  local vy = ((ya - ecs.get(a, 'old_y')) + (yb - ecs.get(b, 'old_y'))) / 2
  local rest = ecs.get(id, 'rest_len') / 2
  local rho = ecs.get(id, 'linear_density') or 0
  local rope = ecs.get(id, 'catenary_a') ~= nil
  local m0 = rope and (JOINT0 / 2 + rho * rest / 2) or JOINT0 / 2
  for _, end_id in ipairs({a, b}) do
    local p = ecs.spawn({x = mx, y = my, old_x = mx - vx, old_y = my - vy, inv_mass = 1 / m0,
      radius = DEBRIS_R, layer = 8, mask = 1, contact = 0, contact_nx = 0, contact_ny = 0,
      damping = DEBRIS_DAMP, mass0 = m0})
    local c = {node_a = end_id, node_b = p, rest_len = rest, compliance = 0, lambda = 0,
      force_max = 0}
    if rope then
      c.mode = 1 c.catenary_a = 0 c.catenary_x0 = 0 c.catenary_y0 = 0 c.sag = 0
      c.linear_density = rho c.min_slack = 0.001 c.tension = 0
    else
      c.mode = 0 c.layer = 4 c.friction = 0.8 c.linear_density = DENS
    end
    ecs.spawn(c)
  end
  ecs.kill(id)
end

local function break_doomed()
  local n, ids = ecs.query('doomed')
  for i = 1, n do break_part(ids[i]) end
  if n > 0 then remass() end
end

-- one tick of looking: a part whose load is over its strength is marked; it breaks at the start
-- of the next tick (break_doomed)
local function judge()
  local n, ids, str, f = ecs.query('strength', 'force_max')
  local m, tids, _, t = ecs.query('strength', 'tension')
  local tension = {}
  for i = 1, m do tension[tids[i]] = t[i] end
  for i = 1, n do
    if math.max(f[i], tension[ids[i]] or 0) > str[i] then ecs.set(ids[i], 'doomed', 1) end
  end
end

-- free joints outside the world are removed with the links on them
local function sweep()
  local n, ids, xs, ys = ecs.query('x', 'y', 'mass0')
  local gone, list = {}, {}
  for i = 1, n do
    if ys[i] > WORLD.y1 or xs[i] < WORLD.x0 or xs[i] > WORLD.x1 then
      gone[ids[i]] = true
      list[#list + 1] = ids[i]
    end
  end
  if #list == 0 then return end
  local k, lids, a, b = ecs.query('node_a', 'node_b')
  for i = 1, k do
    if gone[math.tointeger(a[i])] or gone[math.tointeger(b[i])] then ecs.kill(lids[i]) end
  end
  for _, id in ipairs(list) do ecs.kill(id) end
  remass()
end

local function build()
  -- the road: fixed at both ends, 10 links of 40.5
  point('road_l', 100, 300, 0, 0, 1, 1)
  point('road_r', 500, 300, 0, 0, 1, 1)
  local prev = 'road_l'
  for i = 1, N do
    -- inv_mass is set by remass at the end of build
    point('joint_' .. i, 100 + i * 40, 300, 1, 3, 8, 1, 0.996, JOINT0)
    link('beam_' .. i, prev, 'joint_' .. i, BEAM, 0, 1, 4, beam_strength(i))
    prev = 'joint_' .. i
  end
  link('beam_' .. (N + 1), prev, 'road_r', BEAM, 0, 1, 4, beam_strength(N + 1))
  -- the cable: fixed at both ends, one free joint in the middle
  point('cable_l', 100, 100, 0, 0, 1, 1)
  point('cable_r', 500, 100, 0, 0, 1, 1)
  point('hang', 300, 190, 0.1, 3, 8, 1, 0.996)
  cable('cable_a', 'cable_l', 'hang', 220, 2000000)
  cable('cable_b', 'hang', 'cable_r', 220, 1900000)
  -- the only hanger: a rope (it sags when slack, it never pushes) from the cable joint to the
  -- middle of the road. a strut (mode 2) would be the one that gets compressed
  cable('hanger', 'hang', 'joint_5', 110, 1600000)
  -- the wheel and the mouse: a soft spring of length 0 pulls the wheel to the mouse
  point('wheel', 200, 250, 0.02, 12, 16, 23, 0.984)
  point('mouse', 200, 250, 0, 0, 32, 0)
  link('drag', 'mouse', 'wheel', 0, 0.00002, 0, nil)
  remass()
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
  break_doomed()
  ecs.fill('force_max', 0)
  for i = 1, 16 do substep(dt / 16) end
  ecs.update_catenary(980)
  if ecs.tick() > WARMUP then judge() end
  sweep()
end
