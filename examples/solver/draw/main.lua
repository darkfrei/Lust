-- draws the scene: the ground, the road links, the joints, the ball and the cable (a curve
-- from ecs.sample_catenary). draw code only reads
return function()
  gfx.size(800, 600)
  gfx.clear('#10203a')
  gfx.line(0, 500, 800, 500, 2, '#6c8')
  local n, ids, ax, ay = ecs.query('x', 'y')
  for i = 1, n do
    gfx.circle(ax[i], ay[i], 4, '#9cf')
  end
  local k, lids, la, lb = ecs.query('node_a', 'node_b')
  for i = 1, k do
    local xa = ecs.get(la[i], 'x')
    local ya = ecs.get(la[i], 'y')
    local xb = ecs.get(lb[i], 'x')
    local yb = ecs.get(lb[i], 'y')
    if xa and xb then
      if ecs.get(lids[i], 'catenary_a') then
        local px, py = xa, ya
        for s = 1, 16 do
          local cx, cy = ecs.sample_catenary(lids[i], s / 16)
          gfx.line(px, py, cx, cy, 1, '#fff')
          px, py = cx, cy
        end
      else
        gfx.line(xa, ya, xb, yb, 3, '#c96')
      end
    end
  end
  local bx, by = ecs.get('ball', 'x'), ecs.get('ball', 'y')
  if bx then gfx.circle(bx, by, 12, '#f66') end
end
