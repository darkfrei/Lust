-- draws the bridge. fixed points (rigid fixings) are orange squares, free joints are blue
-- circles, the cable is the white curve, the road is tan, the hanger is pink, the wheel is
-- red, the mouse marker is a small cross. draw code only reads
-- the load of a part against its strength as a color: green when light, yellow at half, red at
-- the strength and beyond. the load of a link is its biggest force (force_max) or, for a cable,
-- the tension of its shape, whichever is bigger
local function load_color(id)
  local strength = ecs.get(id, 'strength')
  if not strength or strength <= 0 then return nil end
  local load = math.max(ecs.get(id, 'force_max') or 0, ecs.get(id, 'tension') or 0)
  local r = math.min(1, load / strength)
  local red, green, blue
  if r < 0.5 then
    local t = r / 0.5
    red, green, blue = 0.25 + 0.75 * t, 0.85, 0.3 - 0.1 * t
  else
    local t = (r - 0.5) / 0.5
    red, green, blue = 1, 0.85 - 0.65 * t, 0.2
  end
  return string.format('#%02x%02x%02x', math.floor(red * 255), math.floor(green * 255), math.floor(blue * 255))
end

return function()
  gfx.size(800, 600)
  gfx.clear('#10203a')
  gfx.line(0, 500, 800, 500, 2, '#6c8')
  local k, lids, la, lb = ecs.query('node_a', 'node_b')
  for i = 1, k do
    -- link ends are numbers in a num column (floats): ecs.get wants an integer id
    local ea, eb = math.tointeger(la[i]), math.tointeger(lb[i])
    local xa, ya = ecs.get(ea, 'x'), ecs.get(ea, 'y')
    local xb, yb = ecs.get(eb, 'x'), ecs.get(eb, 'y')
    if xa and xb then
      if ecs.get(lids[i], 'catenary_a') then
        -- a rope: the curve of ecs.sample_catenary. a slack rope hanging straight down folds on
        -- itself (the shape goes down and comes back up along one line); to be seen the way
        -- back is drawn 5 px to the right (a drawing choice only)
        local xs, ys, low = {xa}, {ya}, 1
        for s = 1, 16 do
          local cx, cy = ecs.sample_catenary(lids[i], s / 16)
          xs[#xs + 1], ys[#ys + 1] = cx, cy
          if cy > ys[low] then low = #xs end
        end
        local fold = ecs.get(lids[i], 'catenary_a') == 0 and ecs.get(lids[i], 'sag') > 0
        local color = load_color(lids[i]) or '#fff'
        for s = 2, #xs do
          local x1, x2 = xs[s - 1], xs[s]
          if fold and s - 1 >= low then x1 = x1 + 5 end
          if fold and s > low then x2 = x2 + 5 end
          gfx.line(x1, ys[s - 1], x2, ys[s], 3, color)
        end
      elseif ecs.get(lids[i], 'layer') then
        gfx.line(xa, ya, xb, yb, 5, load_color(lids[i]) or '#c96')
      elseif ecs.get(lids[i], 'strength') then
        -- a truss member (not a road): thinner than the road, same load color
        gfx.line(xa, ya, xb, yb, 3, load_color(lids[i]) or '#c96')
      else
        gfx.line(xa, ya, xb, yb, 1, '#456')
      end
    end
  end
  local n, ids, ax, ay, aw, ar, al = ecs.query('x', 'y', 'inv_mass', 'radius', 'layer')
  for i = 1, n do
    if al[i] == 32 then
      gfx.line(ax[i] - 6, ay[i], ax[i] + 6, ay[i], 1, '#8a8')
      gfx.line(ax[i], ay[i] - 6, ax[i], ay[i] + 6, 1, '#8a8')
    elseif aw[i] == 0 then
      gfx.rect(ax[i] - 6, ay[i] - 6, 12, 12, '#fa3')
    elseif al[i] == 16 then
      gfx.circle(ax[i], ay[i], ar[i], '#f66')
    elseif ecs.get(ids[i], 'broken') == 1 then
      -- the new end of a broken half: half the size of a joint, dark brown
      gfx.circle(ax[i], ay[i], 2, '#5a3a1e')
    else
      gfx.circle(ax[i], ay[i], 4, '#9cf')
    end
  end
  -- the legend: the load of a part against its strength
  for i = 0, 10 do
    local r = i / 10
    local red, green, blue
    if r < 0.5 then red, green, blue = 0.25 + 1.5 * r, 0.85, 0.3 - 0.2 * r else red, green, blue = 1, 0.85 - 1.3 * (r - 0.5), 0.2 end
    gfx.rect(20 + i * 14, 566, 14, 10, string.format('#%02x%02x%02x', math.floor(red * 255), math.floor(green * 255), math.floor(blue * 255)))
  end
  gfx.text(20, 552, 12, '#aab', 'load / strength: 0 ... 100 %')
  -- the buttons (a click inside is handled in sys/physics.lua at the same places); keys 1, 2, R do the same
  local level = ecs.get('app', 'level') or 1
  for i, b in ipairs({{480, 90, 'Bridge 1 [1]', level == 1}, {578, 90, 'Bridge 2 [2]', level == 2}, {676, 84, 'Restart [R]', false}}) do
    gfx.rect(b[1], 8, b[2], 24, b[4] and '#3a5a8a' or '#24385a')
    gfx.text(b[1] + 8, 14, 12, '#dde', b[3])
  end
end
