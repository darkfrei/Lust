-- draws the bridge. fixed points (rigid fixings) are orange squares, free joints are blue
-- circles, the cable is the white curve, the road is tan, the hanger is pink, the wheel is
-- red, the mouse marker is a small cross. draw code only reads
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
        local color = math.abs(xa - xb) < 5 and '#f8c' or '#fff'
        for s = 2, #xs do
          local x1, x2 = xs[s - 1], xs[s]
          if fold and s - 1 >= low then x1 = x1 + 5 end
          if fold and s > low then x2 = x2 + 5 end
          gfx.line(x1, ys[s - 1], x2, ys[s], 2, color)
        end
      elseif ecs.get(lids[i], 'layer') then
        gfx.line(xa, ya, xb, yb, 4, '#c96')
      elseif ecs.get(lids[i], 'mode') == 1 then
        gfx.line(xa, ya, xb, yb, 2, '#f8c')
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
    else
      gfx.circle(ax[i], ay[i], 4, '#9cf')
    end
  end
end
