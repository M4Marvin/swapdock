import { createFileRoute } from '@tanstack/react-router'
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { api, type Problem } from '../api'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

export const Route = createFileRoute('/doctor')({
  component: DoctorPage,
})

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}

function counts(problems: Problem[]) {
  return {
    errors: problems.filter((p) => p.severity === 'error').length,
    warnings: problems.filter((p) => p.severity === 'warning').length,
  }
}

function DoctorPage() {
  const [publicUrls, setPublicUrls] = useState(false)
  const query = useQuery({
    queryKey: ['doctor', publicUrls],
    queryFn: ({ signal }) => api.doctorAll(publicUrls, signal),
  })

  const problems = query.data?.problems ?? []
  const { errors, warnings } = counts(problems)

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-end gap-3">
        <div>
          <h1 className="text-xl font-semibold">Doctor</h1>
          <p className="text-sm text-muted-foreground">
            Compose paths, bind mounts, ports, images, healthchecks and
            optional public URLs. Read-only.
          </p>
        </div>
        <span className="flex-1" />
        <label className="flex items-center gap-2 text-sm">
          <Checkbox
            checked={publicUrls}
            onCheckedChange={(checked) => setPublicUrls(checked === true)}
          />
          probe public URLs
        </label>
        <Button
          variant="outline"
          onClick={() => void query.refetch()}
          disabled={query.isFetching}
        >
          {query.isFetching ? 'Checking…' : 'Refresh'}
        </Button>
      </div>

      {query.isError && (
        <div
          role="alert"
          className="rounded-lg border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive"
        >
          {errorText(query.error)}
        </div>
      )}

      {query.isLoading ? (
        <Skeleton className="h-72 w-full" />
      ) : (
        <Card>
          <CardHeader>
            <CardTitle className="flex flex-wrap items-center gap-2">
              Findings
              <Badge variant={errors > 0 ? 'destructive' : 'success'}>
                {errors} error{errors === 1 ? '' : 's'}
              </Badge>
              <Badge variant={warnings > 0 ? 'warning' : 'outline'}>
                {warnings} warning{warnings === 1 ? '' : 's'}
              </Badge>
            </CardTitle>
          </CardHeader>
          <CardContent>
            {problems.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No problems found.
              </p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>severity</TableHead>
                    <TableHead>code</TableHead>
                    <TableHead>app</TableHead>
                    <TableHead>message</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {problems.map((p, i) => (
                    <TableRow key={`${p.code}-${p.app ?? ''}-${i}`}>
                      <TableCell>
                        <Badge
                          variant={
                            p.severity === 'error' ? 'destructive' : 'outline'
                          }
                        >
                          {p.severity}
                        </Badge>
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {p.code}
                      </TableCell>
                      <TableCell className="font-mono text-xs">
                        {p.app ?? '—'}
                      </TableCell>
                      <TableCell className="text-sm">{p.message}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  )
}
