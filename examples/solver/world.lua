-- a hanging bridge of 10 links, a ball on it, the ground below and a cable above: the
-- sample of the solver (ecs.integrate_verlet, solve_distance, project_edges, project_segments,
-- update_catenary). everything the solver reads is a num component of its own
return {
  spec = 'game',
  types = {
    x = 'num', y = 'num', old_x = 'num', old_y = 'num', inv_mass = 'num', radius = 'num',
    layer = 'num', mask = 'num', contact = 'num', contact_nx = 'num', contact_ny = 'num',
    node_a = 'num', node_b = 'num', rest_len = 'num', compliance = 'num', mode = 'num',
    lambda = 'num', force_max = 'num',
    x1 = 'num', y1 = 'num', x2 = 'num', y2 = 'num', friction = 'num',
    catenary_a = 'num', catenary_x0 = 'num', catenary_y0 = 'num', sag = 'num',
    linear_density = 'num', min_slack = 'num', tension = 'num', strength = 'num', damping = 'num', ready = 'num', mass0 = 'num', born = 'num', broken = 'num', weighs = 'num', level = 'num', dscale = 'num', spin = 'num', phase = 'num', doomed = 'num',
  },
  entities = {
    app = {ready = 0, level = 1, dscale = 0.01},
    ground = {x1 = 0, y1 = 500, x2 = 800, y2 = 500, friction = 0.5, layer = 1},
    ['sys.physics'] = {ord = 1},
  },
}
