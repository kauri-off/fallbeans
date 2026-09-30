import {
  bumperRamp,
  coopGate,
  doorRows,
  movingPlatforms,
  pickSections,
  pistons,
  raceCourse,
  rotorDecks,
  timedDoors,
  withRests,
} from '../../sim/course';
import { defineMap } from '../../sim/map';
import meta from './meta';

/**
 * Doors of every kind: rows where only some doors give way, a heavy gate that opens by itself only
 * now and then (or while someone holds a button for the others), doors sliding open and shut on
 * their own rhythms; in between, a few more challenges drawn from the seed; a bumper ramp to the line.
 */
export default defineMap(
  meta,
  (b, ctx) => {
    const middle = pickSections(b.rng, [timedDoors(3), rotorDecks(2), movingPlatforms(5), doorRows(3), pistons(3)], 3);
    return raceCourse(b, ctx, {
      sections: withRests([doorRows(2), coopGate(), ...middle, bumperRamp(4)]),
    });
  },
  ['castle', 'meadow', 'royal'],
);
