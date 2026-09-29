import { defineGame } from '../../shared/game';

export default defineGame({
  id: 'fruit-memory',
  title: 'Фруктовая память',
  genre: 'survival',
  rules: 'survival',
  desc: 'Запомните, где какой фрукт. Когда плитки опустеют, встаньте на нужный — остальные провалятся!',
  goal: 'Стойте на правильном фрукте',
  duration: 100,
});
