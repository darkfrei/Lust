-- after 300 ticks the ball lies on the bridge (not under it, not on the ground), the bridge
-- holds and the links felt a force
step(300)
local by = get('ball', 'y')
assert(by < 480, 'the ball is above the ground line: ' .. by)
assert(by > 250, 'the ball did not fall through the bridge: ' .. by)
for i = 1, 10 do
  assert(get('beam_' .. i, 'force_max') > 0, 'beam ' .. i .. ' feels a force')
end
local a = get('cable', 'catenary_a')
assert(a ~= nil and a ~= 0, 'the cable has a curve')
assert(get('cable', 'sag') > 0, 'the cable sags')
