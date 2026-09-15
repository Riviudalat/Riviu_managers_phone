import {describe,it,expect} from "vitest";
import {asDistribution,changeDistribution,distributionOf} from "./nurtureDistribution";
import type {NurtureSettings} from "./types";
const settings={likeProb:20,saveProb:5,commentProb:2,followProb:1,likeEnabled:true,saveEnabled:true,commentEnabled:true,followEnabled:true} as NurtureSettings;
describe("nurture allocation",()=>{
 it("fills the remainder with viewing and caps an edited action at the remaining budget",()=>{
  expect(distributionOf(settings).watch).toBe(72);
  const next=changeDistribution(settings,"likeProb",99);
  expect(next.likeProb).toBe(92);
  expect(distributionOf(next).total).toBe(100);
  expect(distributionOf(next).watch).toBe(0);
 });
 it("returns disabled share to viewing without losing its saved number",()=>{
  const next={...settings,commentEnabled:false};
  expect(distributionOf(next).watch).toBe(74);expect(next.commentProb).toBe(2);
 });
 it("normalizes an old independent draft without changing the original settings",()=>{
  const old={...settings,likeProb:80,commentProb:80};
  const converted=asDistribution(old);expect(distributionOf(converted).total).toBe(100);
  expect(converted.actionSelection).toBe("exclusive");expect(old.likeProb).toBe(80);
 });
});
