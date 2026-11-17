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
-- the pieces of a broken part end in a new small joint: radius is half the width of a beam (the
-- road is drawn 5 px wide), and how much speed it keeps per substep
local DEBRIS_R = 1.25
local DEBRIS_DAMP = 0.995
-- friction against turning, per substep: SPIN is the share of the turning of a link (its ends
-- moving across it) that is taken, BEND the share of the turning of one link against the next at
-- a joint (hinge friction)
local SPIN = 0.005
local BEND = 0.05
-- the start of the scene is a shock (a straight, taut road and cable begin to hang, forces of
-- 10^7 and more in the first ticks), so nothing is judged before this many ticks after the level was built
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
local function link(name, a, b, rest, comp, mode, layer, strength, weighs)
  local c = {node_a = ecs.id(a), node_b = ecs.id(b), rest_len = rest, compliance = comp,
    mode = mode, lambda = 0, force_max = 0, strength = strength}
  -- a road link has a layer (the wheel rides on it). a beam (weighs) has a weight per length
  if layer then c.layer = layer c.friction = 0.8 end
  if weighs then c.linear_density = DENS c.weighs = 1 end
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

-- every beam has a strength of its own: 4.0 to 5.6 million, a different one each
local function beam_strength(i)
  return 4000000 + 400000 * ((i * 7) % 5)
end

-- the mass of every free joint = its own mass0 + half the weight (length * linear_density) of
-- each beam (a link with weighs) on it. cables are not beams here: the solver loads their ends with their weight
local function remass()
  local n, ids, m0 = ecs.query('mass0')
  local sum = {}
  for i = 1, n do sum[ids[i]] = m0[i] end
  local k, _, a, b, rest, dens = ecs.query('node_a', 'node_b', 'rest_len', 'linear_density', 'weighs')
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
      damping = DEBRIS_DAMP, mass0 = m0, broken = 1})
    local c = {node_a = end_id, node_b = p, rest_len = rest, compliance = 0, lambda = 0,
      force_max = 0}
    if rope then
      c.mode = 1 c.catenary_a = 0 c.catenary_x0 = 0 c.catenary_y0 = 0 c.sag = 0
      c.linear_density = rho c.min_slack = 0.001 c.tension = 0
    else
      c.mode = 0 c.layer = ecs.get(id, 'layer') c.linear_density = DENS c.weighs = 1
      if c.layer then c.friction = 0.8 end
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
  -- only the part with the biggest load against its strength is marked: when one link of a chain
  -- goes, the shock overloads its neighbours for a moment, and they must not all go with it
  local worst, ratio = nil, 1
  for i = 1, n do
    local r = math.max(f[i], tension[ids[i]] or 0) / str[i]
    if r > ratio then worst, ratio = ids[i], r end
  end
  if worst then ecs.set(worst, 'doomed', 1) end
end

-- restart or change of level: every point and link goes, build makes the level again
local function restart(level)
  ecs.set('app', 'level', level)
  local n, ids = ecs.query('node_a')
  for i = 1, n do ecs.kill(ids[i]) end
  n, ids = ecs.query('x', 'y')
  for i = 1, n do ecs.kill(ids[i]) end
  ecs.set('app', 'ready', 0)
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

local function build_suspension()
  -- the road: fixed at both ends, 10 links of 40.5
  point('road_l', 100, 300, 0, 0, 1, 1)
  point('road_r', 500, 300, 0, 0, 1, 1)
  local prev = 'road_l'
  for i = 1, N do
    -- inv_mass is set by remass at the end of build
    point('joint_' .. i, 100 + i * 40, 300, 1, 3, 8, 1, 0.996, JOINT0)
    link('beam_' .. i, prev, 'joint_' .. i, BEAM, 0, 1, 4, beam_strength(i), true)
    prev = 'joint_' .. i
  end
  link('beam_' .. (N + 1), prev, 'road_r', BEAM, 0, 1, 4, beam_strength(N + 1), true)
  -- the cable: fixed at both ends, one free joint in the middle
  point('cable_l', 100, 100, 0, 0, 1, 1)
  point('cable_r', 500, 100, 0, 0, 1, 1)
  point('hang', 300, 190, 0.1, 3, 8, 1, 0.996)
  cable('cable_a', 'cable_l', 'hang', 220, 8000000)
  cable('cable_b', 'hang', 'cable_r', 220, 7600000)
  -- the only hanger: a rope (it sags when slack, it never pushes) from the cable joint to the
  -- middle of the road. a strut (mode 2) would be the one that gets compressed
  cable('hanger', 'hang', 'joint_5', 110, 6400000)
end


-- the second bridge: a truss (road and struts only, no cable). the road is the bottom chord
-- (rigid beams, they carry a push too), over it a top chord, verticals and diagonals, a triangle
-- in every panel. 10 panels of 40, height 60
local function build_truss()
  point('road_l', 100, 300, 0, 0, 1, 1)
  point('road_r', 500, 300, 0, 0, 1, 1)
  local H = 60
  local at = {road_l = {100, 300}, road_r = {500, 300}}
  for i = 1, 9 do
    local x = 100 + i * 40
    point('b' .. i, x, 300, 1, 3, 8, 1, 0.996, JOINT0)
    point('t' .. i, x, 300 - H, 1, 3, 8, 1, 0.996, JOINT0)
    at['b' .. i], at['t' .. i] = {x, 300}, {x, 300 - H}
  end
  local function b(i) if i == 0 then return 'road_l' elseif i == 10 then return 'road_r' end return 'b' .. i end
  local count = 0
  local function member(name, a, c, layer)
    local dx, dy = at[a][1] - at[c][1], at[a][2] - at[c][2]
    count = count + 1
    link(name, a, c, math.sqrt(dx * dx + dy * dy), 0, 0, layer, beam_strength(count), true)
  end
  for i = 0, 9 do member('road_' .. i, b(i), b(i + 1), 4) end
  for i = 1, 8 do member('top_' .. i, 't' .. i, 't' .. (i + 1)) end
  for i = 1, 9 do member('post_' .. i, b(i), 't' .. i) end
  member('diag_0', 'road_l', 't1')
  for i = 1, 8 do
    if i < 5 then member('diag_' .. i, b(i), 't' .. (i + 1)) else member('diag_' .. i, 't' .. i, b(i + 1)) end
  end
  member('diag_9', 't9', 'road_r')
end

-- the wheel and the mouse, the same in every level: a soft spring of length 0 pulls the wheel
-- to the mouse
local function build_wheel()
  point('wheel', 200, 250, 0.02, 12, 16, 23, 0.984)
  point('mouse', 200, 250, 0, 0, 32, 0)
  link('drag', 'mouse', 'wheel', 0, 0.00002, 0, nil)
end

local function build()
  if ecs.get('app', 'level') == 2 then build_truss() else build_suspension() end
  build_wheel()
  remass()
  ecs.set('app', 'born', ecs.tick())
  ecs.set('app', 'ready', 1)
end

local function substep(h)
  ecs.integrate_verlet(h, 0, 980, 0.999)
  ecs.apply_catenary_load(h, 980)
  ecs.solve_distance(h, 1)
  ecs.damp_angular(SPIN, BEND)
  ecs.project_edges(1)
  ecs.project_segments(1)
end

return function(dt)
  local level = ecs.get('app', 'level') or 1
  if input.pressed('1') then restart(1) elseif input.pressed('2') then restart(2)
  elseif input.pressed('r') then restart(level) end
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
  if ecs.tick() - ecs.get('app', 'born') > WARMUP then judge() end
  sweep()
end
