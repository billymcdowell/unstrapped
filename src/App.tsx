import type { ReactNode } from "react";
import { AppProviders, useNav, type ScreenId } from "./lib/app-context";
import { Sidebar } from "./shell/Sidebar";
import { Header } from "./shell/Header";
import { Home } from "./screens/Home";
import { Sleep } from "./screens/Sleep";
import { Recovery } from "./screens/Recovery";
import { Strain } from "./screens/Strain";
import { Health } from "./screens/Health";
import { Stress } from "./screens/Stress";
import { Trends } from "./screens/Trends";
import { Journal } from "./screens/Journal";
import { Assessments } from "./screens/Assessments";
import { Healthspan } from "./screens/Healthspan";
import { Data } from "./screens/Data";
import { Strap } from "./screens/Strap";

const SCREENS: Record<ScreenId, () => ReactNode> = {
  home: () => <Home />, sleep: () => <Sleep />, recovery: () => <Recovery />, strain: () => <Strain />,
  health: () => <Health />, stress: () => <Stress />, trends: () => <Trends />, journal: () => <Journal />,
  assessments: () => <Assessments />, healthspan: () => <Healthspan />, data: () => <Data />, strap: () => <Strap />,
};

function Shell() {
  const { screen } = useNav();
  return (
    <div className="shell">
      <Sidebar />
      <main className="main">
        <Header />
        <div className="content">{SCREENS[screen]()}</div>
      </main>
    </div>
  );
}

export default function App() {
  return (
    <AppProviders>
      <Shell />
    </AppProviders>
  );
}
